use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::warn;

use jp_core::knowledge::dictionaries::{self, READER_FREQUENCY};
use jp_core::knowledge::vocabulary::{self, Status, Term};
use jp_mine_core::card;
use jp_mine_core::lookup::{bold_target_in_sentence, target_surface};

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::routes::mining::{build_sentence_views, format_seconds};
use crate::services::export::ExportSentence;
use crate::services::media::media_filenames;
use crate::services::pipeline;

// --- Request/response types ---

#[derive(Deserialize)]
pub struct SubmitRequest {
    url: String,
}

#[derive(Serialize)]
pub struct SubmitResponse {
    video_id: String,
}

#[derive(Serialize)]
struct JobResponse {
    job_id: i64,
    video_id: String,
    video_title: Option<String>,
    status: String,
    is_terminal: bool,
    error_message: Option<String>,
    progress_percent: Option<u8>,
    sentence_count: usize,
    sentences: Vec<SentenceJson>,
}

#[derive(Serialize)]
struct SentenceJson {
    id: i64,
    timestamp: String,
    start_seconds: u64,
    text: String,
    tokens: Vec<TokenJson>,
}

#[derive(Serialize)]
struct TokenJson {
    surface: String,
    base_form: String,
    is_content_word: bool,
    reading: String,
    start: usize,
    status: String,
}

#[derive(Deserialize, Default)]
pub struct PollQuery {
    #[serde(default)]
    sc: Option<usize>,
    #[serde(default)]
    st: Option<String>,
}

#[derive(Deserialize)]
pub struct DefineQuery {
    /// The ledger's headword, not the surface — a click sends the pair the
    /// tokenizer decided on, so 振っ is defined as 振る.
    pub term: String,
    pub reading: Option<String>,
}

#[derive(Deserialize)]
pub struct ExpandQuery {
    /// The line from the clicked word's first character to its end.
    pub text: String,
}

#[derive(Deserialize)]
pub struct JudgeRequest {
    /// The ledger key, never the surface.
    pub headword: String,
    pub reading: String,
    /// `known` or `unknown`. `new` and `seen` are not reachable by hand.
    pub status: String,
}

#[derive(Deserialize)]
pub struct ExportRequest {
    job_id: i64,
    sentences: Vec<ExportSentenceRequest>,
}

#[derive(Deserialize)]
pub struct ExportSentenceRequest {
    id: i64,
    #[serde(default)]
    target_word: Option<String>,
    /// The ledger reading for `target_word`, as the popup had it.
    ///
    /// Sent rather than re-derived, because a word picked out of the popup's
    /// scan need not be a token at all: 経年劣化 is a compound the tokenizer
    /// split, so looking its reading up among the tokens finds nothing and the
    /// card loses its pitch and its furigana.
    #[serde(default)]
    target_reading: Option<String>,
}

#[derive(Serialize)]
struct ExportResponse {
    count: usize,
    exported_ids: Vec<i64>,
}

#[derive(Serialize)]
struct ExportErrorResponse {
    error: String,
}

// --- Handlers ---

pub async fn submit_job(
    State(state): State<AppState>,
    Json(body): Json<SubmitRequest>,
) -> Result<Response, AppError> {
    use crate::services::download::{extract_video_id, is_valid_youtube_url};

    let url = body.url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::BadRequest("URL is required".into()));
    }
    if !is_valid_youtube_url(&url) {
        return Err(AppError::BadRequest("not a valid YouTube URL".into()));
    }

    let video_id = extract_video_id(&url)
        .ok_or_else(|| AppError::BadRequest("could not extract video ID from URL".into()))?;

    // Reuse existing non-error job
    if db::get_job_by_video_id(&state.db, &video_id)
        .await?
        .is_some()
    {
        return Ok(Json(SubmitResponse { video_id }).into_response());
    }

    let job_id = db::create_job(&state.db, &url, &video_id).await?;

    let pool = state.db.clone();
    let downloader = Arc::clone(&state.downloader);
    let transcriber = Arc::clone(&state.transcriber);
    let audio_dir = state.audio_dir.clone();

    tokio::spawn(async move {
        pipeline::process_job(pool, job_id, url, audio_dir, downloader, transcriber).await;
    });

    Ok((StatusCode::CREATED, Json(SubmitResponse { video_id })).into_response())
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(video_id): Path<String>,
) -> Result<Response, AppError> {
    let job = db::get_latest_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let (sentence_views, max_end) = build_sentence_views(&state, job.id).await?;

    let progress_percent = job.video_duration.map(|d| {
        if d > 0.0 {
            (max_end / d * 100.0).min(100.0) as u8
        } else {
            0
        }
    });

    let sentence_count = sentence_views.len();
    let sentences = sentence_views.into_iter().map(sentence_to_json).collect();

    Ok(Json(JobResponse {
        job_id: job.id,
        video_id,
        video_title: job.video_title,
        status: job.status.as_str().to_string(),
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
        progress_percent,
        sentence_count,
        sentences,
    })
    .into_response())
}

pub async fn poll_status(
    State(state): State<AppState>,
    Path(video_id): Path<String>,
    Query(poll): Query<PollQuery>,
) -> Result<Response, AppError> {
    let job = db::get_latest_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let status_str = job.status.as_str().to_string();

    // Return 204 if nothing changed
    if let (Some(prev_sc), Some(prev_st)) = (poll.sc, &poll.st) {
        let current_count = db::count_sentences_for_job(&state.db, job.id).await? as usize;
        if prev_st == &status_str && prev_sc == current_count {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }

    let (sentence_views, max_end) = build_sentence_views(&state, job.id).await?;

    let progress_percent = job.video_duration.map(|d| {
        if d > 0.0 {
            (max_end / d * 100.0).min(100.0) as u8
        } else {
            0
        }
    });

    let sentence_count = sentence_views.len();
    let sentences = sentence_views.into_iter().map(sentence_to_json).collect();

    Ok(Json(JobResponse {
        job_id: job.id,
        video_id,
        video_title: job.video_title,
        status: status_str,
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
        progress_percent,
        sentence_count,
        sentences,
    })
    .into_response())
}

/// `GET /api/define?term=<headword>&reading=<reading>`
///
/// The overlay's own popup endpoint, minus the lookup recording: a lookup is a
/// reading-session event and there is no session here.
pub async fn define(
    State(state): State<AppState>,
    Query(q): Query<DefineQuery>,
) -> Result<Json<jp_core::define::Definition>, AppError> {
    Ok(Json(
        jp_core::define::define(state.knowledge.pool(), &q.term, q.reading.as_deref()).await?,
    ))
}

/// `GET /api/expand?text=<rest of the line>` — the other readings of a
/// position, for when the tokenizer split a word or picked the wrong reading.
pub async fn expand(
    State(state): State<AppState>,
    Query(q): Query<ExpandQuery>,
) -> Result<Json<Vec<jp_core::define::Expansion>>, AppError> {
    Ok(Json(
        jp_core::define::expand(&state.knowledge, state.highlighter.clone(), &q.text).await?,
    ))
}

/// `POST /api/judge` — mark a word known or unknown, in the shared ledger.
///
/// The same write `#read`'s tap makes, against the same rows: a word judged
/// while mining a video is judged, and the reading view stops marking it.
pub async fn judge(
    State(state): State<AppState>,
    Json(req): Json<JudgeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let status = Status::parse(&req.status);
    if !matches!(status, Status::Known | Status::Unknown) {
        return Err(AppError::BadRequest(format!(
            "not a judgement: {}",
            req.status
        )));
    }
    let term = Term::new(req.headword, &req.reading);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    vocabulary::set_status(&state.knowledge, &term, status, ts).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn export_sentences(
    State(state): State<AppState>,
    Json(body): Json<ExportRequest>,
) -> Result<Response, AppError> {
    if body.sentences.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(ExportErrorResponse {
                error: "No sentences selected.".into(),
            }),
        )
            .into_response());
    }

    let sentence_ids: Vec<i64> = body.sentences.iter().map(|s| s.id).collect();
    let target_word_map: std::collections::HashMap<i64, (String, Option<String>)> = body
        .sentences
        .iter()
        .filter_map(|s| {
            s.target_word
                .clone()
                .map(|w| (s.id, (w, s.target_reading.clone())))
        })
        .collect();

    let sentences = db::get_sentences_by_ids(&state.db, &sentence_ids).await?;
    if sentences.is_empty() {
        return Err(AppError::NotFound);
    }

    let job = db::get_job(&state.db, body.job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let source = job.video_title.unwrap_or_else(|| job.youtube_url.clone());

    let mut export_sentences = Vec::with_capacity(sentences.len());
    let mut exported_ids = Vec::with_capacity(sentences.len());
    for sentence in sentences {
        exported_ids.push(sentence.id);
        let (screenshot_filename, audio_filename) = media_filenames(sentence.job_id, sentence.id);
        let screenshot_path = format!("{}/{screenshot_filename}", state.media_dir);
        let audio_clip_path = format!("{}/{audio_filename}", state.media_dir);

        let mut screenshot_result = None;
        if let Some(video_path) = &job.video_path {
            let midpoint = (sentence.start_time + sentence.end_time) / 2.0;
            match state
                .media_extractor
                .extract_screenshot(video_path, midpoint, &screenshot_path)
                .await
            {
                Ok(()) => screenshot_result = Some(screenshot_path),
                Err(e) => warn!(
                    sentence_id = sentence.id,
                    error = %e,
                    "screenshot extraction failed, exporting without image"
                ),
            }
        }

        let mut audio_result = None;
        if let Some(audio_path) = &job.audio_path {
            match state
                .media_extractor
                .extract_audio_clip(
                    audio_path,
                    sentence.start_time,
                    sentence.end_time,
                    &audio_clip_path,
                )
                .await
            {
                Ok(()) => audio_result = Some(audio_clip_path),
                Err(e) => warn!(
                    sentence_id = sentence.id,
                    error = %e,
                    "audio clip extraction failed, exporting without audio"
                ),
            }
        }

        let (target_word, sent_reading) = match target_word_map.get(&sentence.id) {
            Some((w, r)) => (Some(w.clone()), r.clone()),
            None => (None, None),
        };

        // Tokenized once and shared: the bolded sentence and the gloss's
        // spelling of the target are two questions about the same analysis.
        let tokens = target_word
            .as_ref()
            .and_then(|_| state.tokenizer.tokenize(&sentence.text).ok());

        // The card's fields, built by `jp_mine_core::card` — the same builders
        // read-stats mines with, so a transcript card and a VN card are one
        // note type rather than two shapes of it.
        //
        // The ledger's own reading for the pair, not the raw one Sudachi gave:
        // `Term::new` lowers it to hiragana and blanks it for a kana headword,
        // and read-stats mines with exactly that. Asking the dictionary a
        // differently-shaped question is how a card silently loses its pitch.
        let reading = sent_reading.unwrap_or_else(|| {
            target_word
                .as_ref()
                .zip(tokens.as_deref())
                .and_then(|(word, tokens)| tokens.iter().find(|t| &t.base_form == word))
                .map(|t| Term::new(t.base_form.clone(), &t.reading).reading)
                .unwrap_or_default()
        });

        let (definition, vocab_furigana, vocab_pitch_num, vocab_pitch_pattern, vocab_frequency) =
            match &target_word {
                Some(word) => {
                    let pool = state.knowledge.pool();
                    let accent = card::accent(pool, word, &reading).await.unwrap_or(None);
                    (
                        card::glossary(pool, word, &reading).await.ok(),
                        Some(card::furigana(word, &reading)),
                        accent.map(card::pitch_num),
                        accent.map(|a| card::pitch_pattern(&reading, a)),
                        reader_rank(pool, word).await,
                    )
                }
                None => (None, None, None, None, None),
            };

        let sentence_html = target_word
            .as_ref()
            .zip(tokens.as_deref())
            .and_then(|(word, tokens)| bold_target_in_sentence(tokens, word));

        let mut compact_def = None;
        if let (Some(word), Some(definer)) = (&target_word, &state.llm_definer) {
            // Rated on the spelling the video used, not the base form the click
            // selected — 饐える prices its kanji where its own sentence's
            // すえた does not. The bolded sentence is what carries that span.
            let written = tokens
                .as_deref()
                .and_then(|t| target_surface(t, word))
                .unwrap_or_else(|| word.clone());
            let marked = sentence_html.as_deref().unwrap_or(&sentence.text);
            match definer.define(&written, marked).await {
                Ok(def) => compact_def = Some(def),
                Err(e) => warn!(word, error = %e, "CompactDef failed, exporting without"),
            }
        }

        export_sentences.push(ExportSentence {
            source: format!("{source} ({})", format_seconds(sentence.start_time)),
            sentence_text: sentence.text,
            screenshot_path: screenshot_result,
            audio_clip_path: audio_result,
            target_word,
            definition,
            vocab_furigana,
            vocab_pitch_num,
            vocab_pitch_pattern,
            vocab_frequency,
            sentence_html,
            compact_def,
        });
    }

    match state.exporter.export_sentences(export_sentences).await {
        Ok(count) => Ok(Json(ExportResponse {
            count,
            exported_ids,
        })
        .into_response()),
        Err(e) => {
            let raw = e.to_string();
            warn!(error = %raw, "export to Anki failed");
            let message = if raw.contains("connection")
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
                Json(ExportErrorResponse {
                    error: message.into(),
                }),
            )
                .into_response())
        }
    }
}

pub async fn sentence_audio(
    State(state): State<AppState>,
    Path((video_id, sentence_id)): Path<(String, i64)>,
) -> Result<Response, AppError> {
    let job = db::get_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let sentences = db::get_sentences_by_ids(&state.db, &[sentence_id]).await?;
    let sentence = sentences.into_iter().next().ok_or(AppError::NotFound)?;
    if sentence.job_id != job.id {
        return Err(AppError::NotFound);
    }
    let audio_path = job
        .audio_path
        .ok_or(AppError::BadRequest("no audio available".into()))?;

    let (_, audio_filename) = media_filenames(job.id, sentence_id);
    let clip_path = format!("{}/{audio_filename}", state.media_dir);

    if !tokio::fs::try_exists(&clip_path).await.unwrap_or(false) {
        tokio::fs::create_dir_all(&state.media_dir)
            .await
            .map_err(|e| AppError::Media(format!("failed to create media dir: {e}")))?;

        state
            .media_extractor
            .extract_audio_clip(
                &audio_path,
                sentence.start_time,
                sentence.end_time,
                &clip_path,
            )
            .await
            .map_err(|e| AppError::Media(e.to_string()))?;
    }

    let bytes = tokio::fs::read(&clip_path)
        .await
        .map_err(|e| AppError::Media(format!("failed to read audio clip: {e}")))?;

    Ok(([(axum::http::header::CONTENT_TYPE, "audio/mpeg")], bytes).into_response())
}

// --- Helpers ---

/// How common the word is in fiction — the rank the reader's own tools use.
async fn reader_rank(pool: &sqlx::SqlitePool, term: &str) -> Option<i64> {
    match dictionaries::by_title(pool, READER_FREQUENCY).await {
        Ok(Some(d)) => dictionaries::lookup_frequency(pool, d.id, term)
            .await
            .unwrap_or(None),
        _ => None,
    }
}

fn sentence_to_json(view: crate::routes::mining::SentenceView) -> SentenceJson {
    SentenceJson {
        id: view.id,
        timestamp: view.timestamp,
        start_seconds: view.start_seconds,
        text: view.text,
        tokens: view
            .tokens
            .into_iter()
            .map(|t| TokenJson {
                surface: t.surface,
                base_form: t.base_form,
                is_content_word: t.is_content_word,
                reading: t.reading,
                start: t.start,
                status: t.status,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
