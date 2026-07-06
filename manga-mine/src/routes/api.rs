use std::net::{IpAddr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use jp_core::dictionary::format_furigana;
use jp_core::tokenize::is_content_word;
use jp_mine_core::export::{AnkiConnectExporter, AnkiExporter, ExportSentence};
use jp_mine_core::lookup::{bold_target_in_sentence, lookup_word};

use crate::app::AppState;
use crate::error::AppError;
use crate::services::image_ops::{self, CropRect};
use crate::text::split_sentences;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

// --- Request/response types ---

#[derive(Serialize)]
pub struct QueueResponse {
    photos: Vec<QueuePhoto>,
}

#[derive(Serialize)]
pub struct QueuePhoto {
    name: String,
}

#[derive(Serialize)]
pub struct UploadResponse {
    name: String,
}

#[derive(Deserialize)]
pub struct OcrRequest {
    /// Fractional crop rectangle (0.0–1.0) in the displayed (oriented) image.
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Serialize)]
pub struct OcrResponse {
    text: String,
    sentences: Vec<SentenceJson>,
}

#[derive(Serialize)]
pub struct SentenceJson {
    text: String,
    tokens: Vec<TokenJson>,
}

#[derive(Serialize)]
pub struct TokenJson {
    surface: String,
    base_form: String,
    is_content_word: bool,
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    word: String,
}

#[derive(Serialize)]
pub struct PreviewResponse {
    word: String,
    reading: String,
    pitch_num: Option<String>,
    frequency: Option<i64>,
    definition_html: Option<String>,
}

#[derive(Deserialize)]
pub struct ExportRequest {
    photo: String,
    sentence: String,
    target_word: Option<String>,
    /// Manga title for the card's source/Document field. Falls back to the
    /// photo filename stem when absent.
    source: Option<String>,
}

#[derive(Serialize)]
pub struct SourcesResponse {
    sources: Vec<String>,
}

#[derive(Serialize)]
pub struct ExportResponse {
    count: usize,
}

#[derive(Serialize)]
pub struct ExportErrorResponse {
    error: String,
}

#[derive(Deserialize)]
pub struct MarkRequest {
    /// "processed" or "skipped"
    status: String,
}

// --- Helpers ---

/// Reject path traversal: the name must be a plain file name.
fn safe_name(name: &str) -> Result<&str, AppError> {
    if name.is_empty()
        || name.starts_with('.')
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err(AppError::BadRequest("invalid photo name".into()));
    }
    Ok(name)
}

fn is_image_file(name: &str) -> bool {
    FsPath::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn content_type_for(name: &str) -> &'static str {
    match FsPath::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

/// Resolve a queue photo path, ensuring the file exists in the inbox.
async fn photo_path(state: &AppState, name: &str) -> Result<PathBuf, AppError> {
    let name = safe_name(name)?;
    let path = state.inbox_dir.join(name);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }
    Ok(path)
}

/// Previously used manga titles, most recent first, persisted as a dotfile in
/// the inbox (invisible to the queue listing). Filesystem-as-state (ADR-010).
const SOURCES_FILE: &str = ".sources.json";
const SOURCES_MAX: usize = 30;

async fn read_sources(state: &AppState) -> Vec<String> {
    let path = state.inbox_dir.join(SOURCES_FILE);
    match tokio::fs::read_to_string(&path).await {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Move (or insert) a source name to the front of the remembered list.
async fn remember_source(state: &AppState, name: &str) {
    let mut sources = read_sources(state).await;
    sources.retain(|s| s != name);
    sources.insert(0, name.to_string());
    sources.truncate(SOURCES_MAX);
    let path = state.inbox_dir.join(SOURCES_FILE);
    if let Err(e) = tokio::fs::write(&path, serde_json::to_string(&sources).unwrap_or_default()).await {
        warn!(error = %e, "failed to persist sources list");
    }
}

/// Infallible extractor for the client address — `None` when the server runs
/// without connect info (route tests).
pub struct ClientAddr(Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(ClientAddr(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0),
        ))
    }
}

/// If the requesting client (e.g. the phone) runs its own AnkiConnect, prefer
/// it so the card lands in the collection the user is actually holding.
/// Probes `http://<client-ip>:8765` with a short timeout; loopback clients
/// already hit the configured exporter.
async fn client_anki_url(url: &str) -> bool {
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
    {
        Ok(http) => http,
        Err(_) => return false,
    };
    let probe = serde_json::json!({ "action": "version", "version": 6 });

    // The first packet to a phone on Wi-Fi power-save can take >1s (ARP +
    // radio wake-up), so a single short attempt produces false negatives —
    // retry once.
    for attempt in 1..=2 {
        match http.post(url).json(&probe).send().await {
            Ok(response) => {
                let is_anki = response
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|body| body.get("result").map(|r| !r.is_null()))
                    .unwrap_or(false);
                if !is_anki {
                    info!(%url, "client responded on 8765 but is not AnkiConnect");
                }
                return is_anki;
            }
            Err(e) => {
                tracing::debug!(%url, attempt, error = %e, "client AnkiConnect probe attempt failed");
            }
        }
    }
    info!(%url, "no client AnkiConnect detected, using configured Anki URL");
    false
}

/// TTL for cached client-AnkiConnect probe results.
const CLIENT_ANKI_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Resolve the exporter for this request: the client's own AnkiConnect when
/// one is (or was recently) detected, the configured default otherwise.
/// Detected exporters are cached per IP so repeat exports skip the probe and
/// reuse the instance (keeping its one-time model/deck setup).
async fn exporter_for_client(
    state: &AppState,
    client: Option<SocketAddr>,
) -> Arc<dyn AnkiExporter> {
    let default = || Arc::clone(&state.exporter);
    if !state.use_client_anki {
        return default();
    }
    let Some(ip) = client.map(|c| c.ip()).filter(|ip| !ip.is_loopback()) else {
        return default();
    };

    let mut cache = state.client_anki_cache.lock().await;
    if let Some(entry) = cache.get(&ip) {
        if entry.checked_at.elapsed() < CLIENT_ANKI_TTL {
            return entry.exporter.clone().unwrap_or_else(default);
        }
    }

    let url = match ip {
        IpAddr::V4(v4) => format!("http://{v4}:8765"),
        IpAddr::V6(v6) => format!("http://[{v6}]:8765"),
    };
    let exporter: Option<Arc<dyn AnkiExporter>> = if client_anki_url(&url).await {
        info!(%url, "using client AnkiConnect");
        Some(Arc::new(AnkiConnectExporter::new(
            url,
            state.anki_config.clone(),
        )))
    } else {
        None
    };

    cache.insert(
        ip,
        crate::app::ClientAnkiEntry {
            checked_at: std::time::Instant::now(),
            exporter: exporter.clone(),
        },
    );
    exporter.unwrap_or_else(default)
}

fn tokenize_sentence(state: &AppState, text: &str) -> SentenceJson {
    let tokens = state
        .tokenizer
        .tokenize(text)
        .map(|toks| {
            toks.into_iter()
                .map(|t| TokenJson {
                    is_content_word: is_content_word(&t.pos),
                    base_form: t.base_form,
                    surface: t.surface,
                })
                .collect()
        })
        .unwrap_or_default();
    SentenceJson {
        text: text.to_string(),
        tokens,
    }
}

// --- Handlers ---

/// The queue is the inbox folder: every image file in it is an un-mined photo,
/// oldest first.
pub async fn list_queue(State(state): State<AppState>) -> Result<Response, AppError> {
    let mut entries = tokio::fs::read_dir(&state.inbox_dir).await?;
    let mut photos: Vec<(std::time::SystemTime, String)> = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !is_image_file(&name) {
            continue;
        }
        let meta = entry.metadata().await?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        photos.push((modified, name));
    }

    photos.sort();
    Ok(Json(QueueResponse {
        photos: photos
            .into_iter()
            .map(|(_, name)| QueuePhoto { name })
            .collect(),
    })
    .into_response())
}

/// Upload an existing photo from the phone gallery into the inbox.
pub async fn upload_photo(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart body: {e}")))?
    {
        let original = field.file_name().unwrap_or("photo.jpg").to_string();
        // Keep only the basename, drop anything suspicious
        let base = FsPath::new(&original)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("photo.jpg")
            .replace(['/', '\\'], "_");
        let base = if is_image_file(&base) { base } else { format!("{base}.jpg") };

        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to read upload: {e}")))?;
        if data.is_empty() {
            continue;
        }

        // Avoid clobbering an existing queue item with the same name
        let mut name = base.clone();
        let mut counter = 1;
        while tokio::fs::try_exists(state.inbox_dir.join(&name)).await.unwrap_or(false) {
            let stem = FsPath::new(&base).file_stem().unwrap_or_default().to_string_lossy();
            let ext = FsPath::new(&base).extension().unwrap_or_default().to_string_lossy();
            name = format!("{stem}-{counter}.{ext}");
            counter += 1;
        }

        tokio::fs::write(state.inbox_dir.join(&name), &data).await?;
        return Ok((StatusCode::CREATED, Json(UploadResponse { name })).into_response());
    }

    Err(AppError::BadRequest("no file in upload".into()))
}

pub async fn get_photo(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, AppError> {
    let path = photo_path(&state, &name).await?;
    let bytes = tokio::fs::read(&path).await?;
    Ok((
        [
            (CONTENT_TYPE, content_type_for(&name)),
            (CACHE_CONTROL, "max-age=3600"),
        ],
        bytes,
    )
        .into_response())
}

pub async fn get_thumbnail(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, AppError> {
    let path = photo_path(&state, &name).await?;
    let bytes = tokio::fs::read(&path).await?;
    let thumb = tokio::task::spawn_blocking(move || image_ops::thumbnail(&bytes, 480))
        .await
        .map_err(|e| AppError::Image(format!("thumbnail task failed: {e}")))?
        .map_err(|e| AppError::Image(e.to_string()))?;
    Ok((
        [
            (CONTENT_TYPE, "image/jpeg"),
            (CACHE_CONTROL, "max-age=3600"),
        ],
        thumb,
    )
        .into_response())
}

/// Crop the user-drawn region, OCR it, and return the text split into
/// tokenized sentences. Nothing is persisted — crop coords and OCR text are
/// transient (ADR-010).
pub async fn ocr_crop(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<OcrRequest>,
) -> Result<Response, AppError> {
    let started = std::time::Instant::now();
    let path = photo_path(&state, &name).await?;
    let bytes = tokio::fs::read(&path).await?;
    let read_ms = started.elapsed().as_millis() as u64;

    let crop_started = std::time::Instant::now();
    let rect = CropRect { x: body.x, y: body.y, w: body.w, h: body.h };
    let crop = tokio::task::spawn_blocking(move || image_ops::crop_for_ocr(&bytes, rect))
        .await
        .map_err(|e| AppError::Image(format!("crop task failed: {e}")))?
        .map_err(|e| AppError::Image(e.to_string()))?;
    let crop_ms = crop_started.elapsed().as_millis() as u64;

    let ocr_started = std::time::Instant::now();
    let text = state
        .ocr
        .recognize(crop)
        .await
        .map_err(|e| AppError::Ocr(e.to_string()))?;
    let ocr_ms = ocr_started.elapsed().as_millis() as u64;

    let tokenize_started = std::time::Instant::now();
    let sentences = split_sentences(&text)
        .iter()
        .map(|s| tokenize_sentence(&state, s))
        .collect();
    let tokenize_ms = tokenize_started.elapsed().as_millis() as u64;

    info!(
        read_ms,
        crop_ms,
        ocr_ms,
        tokenize_ms,
        total_ms = started.elapsed().as_millis() as u64,
        "ocr timing"
    );
    Ok(Json(OcrResponse { text, sentences }).into_response())
}

pub async fn list_sources(State(state): State<AppState>) -> Result<Response, AppError> {
    let sources = read_sources(&state).await;
    Ok(Json(SourcesResponse { sources }).into_response())
}

pub async fn word_preview(
    State(state): State<AppState>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, AppError> {
    let result = lookup_word(&state.dictionaries, &query.word).await;
    Ok(Json(PreviewResponse {
        word: query.word,
        reading: result.reading,
        pitch_num: result.pitch_num,
        frequency: result.frequency,
        definition_html: result.definition_html,
    })
    .into_response())
}

/// Export one card: target word + its sentence + definition + the whole photo
/// (compressed) as the card image (ADR-006). No audio (ADR-005).
pub async fn export_card(
    State(state): State<AppState>,
    ClientAddr(client): ClientAddr,
    Json(body): Json<ExportRequest>,
) -> Result<Response, AppError> {
    let started = std::time::Instant::now();
    let sentence_text = body.sentence.trim().to_string();
    if sentence_text.is_empty() {
        return Err(AppError::BadRequest("sentence is required".into()));
    }

    let path = photo_path(&state, &body.photo).await?;
    let bytes = tokio::fs::read(&path).await?;
    let read_ms = started.elapsed().as_millis() as u64;

    // Compress the whole photo for the card image
    let compress_started = std::time::Instant::now();
    let (max_dim, quality) = (state.card_image_max_dim, state.card_image_quality);
    let compressed =
        tokio::task::spawn_blocking(move || image_ops::compress_photo(&bytes, max_dim, quality))
            .await
            .map_err(|e| AppError::Image(format!("compress task failed: {e}")))?
            .map_err(|e| AppError::Image(e.to_string()))?;
    let compress_ms = compress_started.elapsed().as_millis() as u64;
    let compressed_kb = compressed.len() as u64 / 1024;

    let stem = FsPath::new(&body.photo)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("photo")
        .to_string();
    // Card source = manga title when given, photo name stem otherwise
    let source = body
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let image_filename = format!("manga-mine_{stem}_{millis}.jpg");
    let image_path = format!("{}/{image_filename}", state.media_dir);
    tokio::fs::write(&image_path, &compressed).await?;

    let target_word = body.target_word.filter(|w| !w.trim().is_empty());

    let (definition, vocab_furigana, vocab_pitch_num, vocab_frequency) =
        if let Some(word) = &target_word {
            let result = lookup_word(&state.dictionaries, word).await;
            let furigana = format_furigana(word, &result.reading);
            (
                result.definition_html,
                Some(furigana),
                result.pitch_num,
                result.frequency,
            )
        } else {
            (None, None, None, None)
        };

    let sentence_html = target_word.as_ref().and_then(|word| {
        state
            .tokenizer
            .tokenize(&sentence_text)
            .ok()
            .and_then(|tokens| bold_target_in_sentence(&tokens, word))
    });

    let export = ExportSentence {
        sentence_text,
        source: source.clone().unwrap_or(stem),
        screenshot_path: Some(image_path.clone()),
        audio_clip_path: None,
        target_word,
        definition,
        vocab_furigana,
        vocab_pitch_num,
        vocab_frequency,
        sentence_html,
        llm_definition: None,
    };

    // Prefer the client's own AnkiConnect (phone) when it's up
    let probe_started = std::time::Instant::now();
    let exporter = exporter_for_client(&state, client).await;
    let probe_ms = probe_started.elapsed().as_millis() as u64;

    let anki_started = std::time::Instant::now();
    match exporter.export_sentences(vec![export]).await {
        Ok(count) => {
            info!(
                read_ms,
                compress_ms,
                compressed_kb,
                probe_ms,
                anki_ms = anki_started.elapsed().as_millis() as u64,
                total_ms = started.elapsed().as_millis() as u64,
                "export timing"
            );
            if let Some(name) = &source {
                remember_source(&state, name).await;
            }
            // The image now lives in Anki's media folder — drop the temp copy
            if let Err(e) = tokio::fs::remove_file(&image_path).await {
                warn!(error = %e, path = image_path, "failed to remove temp card image");
            }
            Ok(Json(ExportResponse { count }).into_response())
        }
        Err(e) => {
            let raw = e.to_string();
            warn!(error = %raw, "export to Anki failed");
            let message = if raw.contains("duplicate") {
                "Anki already has a card for this word (duplicate)."
            } else if raw.contains("connection")
                || raw.contains("connect")
                || raw.contains("refused")
                || raw.contains("dns")
            {
                "Could not connect to Anki. Is AnkiConnect running?"
            } else {
                "Export to Anki failed."
            };
            Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExportErrorResponse { error: message.into() }),
            )
                .into_response())
        }
    }
}

/// Marking a photo mined or skipped deletes it — the original still lives in
/// the phone's gallery and the compressed copy lives in Anki, so keeping
/// full-quality photos on the server only grows the inbox (amends ADR-010's
/// file-move with deletion; the queue-is-the-folder model is unchanged).
pub async fn mark_photo(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<MarkRequest>,
) -> Result<Response, AppError> {
    if !matches!(body.status.as_str(), "processed" | "skipped") {
        return Err(AppError::BadRequest(format!(
            "invalid status '{}' (expected 'processed' or 'skipped')",
            body.status
        )));
    }

    let src = photo_path(&state, &name).await?;
    tokio::fs::remove_file(&src).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests;
