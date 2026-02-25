use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;

use tracing::warn;

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::models::JobStatus;
use crate::services::export::ExportSentence;
use crate::services::media::media_filenames;
use crate::services::pipeline;
use crate::services::dictionary::{Dictionary, format_furigana};
use crate::services::tokenize::{Token, is_content_word};

/// Form extractor that uses `serde_html_form` to handle repeated keys
/// (e.g. checkboxes: `sentence_ids=1&sentence_ids=2` -> `Vec<i64>`).
/// Standard `axum::Form` uses `serde_urlencoded` which doesn't support this.
pub(crate) struct HtmlForm<T>(T);

impl<S, T> axum::extract::FromRequest<S> for HtmlForm<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(
        req: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        let value = serde_html_form::from_bytes(&bytes)
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        Ok(HtmlForm(value))
    }
}

// --- Template structs ---
// askama::Template does the rendering, askama_web::WebTemplate adds IntoResponse.

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/submit.html")]
struct SubmitTemplate;

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/job_status.html")]
struct VideoPageTemplate {
    video_id: String,
    job_id: i64,
    video_title: Option<String>,
    status: String,
    is_done: bool,
    is_terminal: bool,
    error_message: Option<String>,
    sentence_count: usize,
    sentences: Vec<SentenceView>,
    segments_found: i64,
    /// Passed through to the included fragment. Always false for the full page
    /// (the page already has its own h2).
    oob_title: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/job_content_fragment.html")]
struct VideoContentFragmentTemplate {
    video_id: String,
    job_id: i64,
    video_title: Option<String>,
    status: String,
    is_done: bool,
    is_terminal: bool,
    error_message: Option<String>,
    sentence_count: usize,
    sentences: Vec<SentenceView>,
    segments_found: i64,
    /// When true, emit an OOB `<h2>` swap to update the video title.
    /// Only set for htmx polling responses, not the initial page include.
    oob_title: bool,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/export_result_fragment.html")]
struct ExportResultTemplate {
    count: usize,
    exported_ids: Vec<i64>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/export_error_fragment.html")]
struct ExportErrorTemplate {
    message: String,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/preview_fragment.html")]
struct PreviewFragmentTemplate {
    video_id: String,
    sentence_id: i64,
    word: String,
    reading: String,
    pitch_num: Option<String>,
    definition_html: Option<String>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/llm_definition_fragment.html")]
struct LlmDefinitionFragmentTemplate {
    definition: Option<String>,
}

struct SentenceView {
    id: i64,
    timestamp: String,
    start_seconds: u64,
    tokens: Vec<TokenView>,
    /// Raw text for display during transcription (no tokenization needed).
    text: String,
}

struct TokenView {
    surface: String,
    base_form: String,
    is_content_word: bool,
}

// --- Form structs ---

#[derive(serde::Deserialize, serde::Serialize)]
pub struct SubmitForm {
    url: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ExportForm {
    job_id: i64,
    #[serde(default)]
    sentence_ids: Vec<i64>,
    /// Repeated field of "sentence_id:base_form" pairs from hidden inputs.
    #[serde(default)]
    target_words: Vec<String>,
}

impl ExportForm {
    /// Parse target_words entries ("id:word") into a lookup map.
    fn target_word_map(&self) -> std::collections::HashMap<i64, String> {
        self.target_words
            .iter()
            .filter_map(|entry| {
                let (id, word) = entry.split_once(':')?;
                let id: i64 = id.parse().ok()?;
                if word.is_empty() {
                    None
                } else {
                    Some((id, word.to_string()))
                }
            })
            .collect()
    }
}

#[derive(serde::Deserialize)]
pub struct PreviewQuery {
    word: String,
}

// --- Handlers ---

pub async fn submit_page() -> impl IntoResponse {
    SubmitTemplate
}

pub async fn submit_youtube(
    State(state): State<AppState>,
    Form(form): Form<SubmitForm>,
) -> Result<Response, AppError> {
    use crate::services::download::{extract_video_id, is_valid_youtube_url};

    let url = form.url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::BadRequest("URL is required".into()));
    }
    if !is_valid_youtube_url(&url) {
        return Err(AppError::BadRequest("not a valid YouTube URL".into()));
    }

    let video_id = extract_video_id(&url)
        .ok_or_else(|| AppError::BadRequest("could not extract video ID from URL".into()))?;

    // Reuse existing non-error job for this video (deduplication)
    if let Some(_existing) = db::get_job_by_video_id(&state.db, &video_id).await? {
        return Ok(Redirect::to(&format!("/{video_id}")).into_response());
    }

    let job_id = db::create_job(&state.db, &url, &video_id).await?;

    let pool = state.db.clone();
    let downloader = Arc::clone(&state.downloader);
    let transcriber = Arc::clone(&state.transcriber);
    let audio_dir = state.audio_dir.clone();

    tokio::spawn(async move {
        pipeline::process_job(pool, job_id, url, audio_dir, downloader, transcriber).await;
    });

    Ok(Redirect::to(&format!("/{video_id}")).into_response())
}

/// Build sentence views for display. During transcription, returns simplified
/// views (text only, no tokenization). For completed jobs, returns fully
/// tokenized views with interactive word selection.
async fn build_sentence_views(
    state: &AppState,
    job_id: i64,
    is_done: bool,
    segments_found: i64,
) -> Result<Vec<SentenceView>, AppError> {
    let show_sentences = is_done || segments_found > 0;
    if !show_sentences {
        return Ok(vec![]);
    }

    let sentences = db::get_sentences_for_job(&state.db, job_id).await?;
    Ok(sentences
        .into_iter()
        .map(|s| {
            // Only tokenize for completed jobs — during transcription we
            // show plain text to keep polling responses fast.
            let tokens = if is_done {
                state
                    .tokenizer
                    .tokenize(&s.text)
                    .map(|toks| {
                        toks.into_iter()
                            .map(|t| TokenView {
                                is_content_word: is_content_word(&t.pos),
                                base_form: t.base_form,
                                surface: t.surface,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };
            SentenceView {
                id: s.id,
                timestamp: format_seconds(s.start_time),
                start_seconds: s.start_time as u64,
                text: s.text,
                tokens,
            }
        })
        .collect())
}

pub async fn video_page(
    State(state): State<AppState>,
    Path(video_id): Path<String>,
) -> Result<Response, AppError> {
    let job = db::get_latest_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let is_done = job.status == JobStatus::Done;
    let segments_found = job.segments_found;
    let sentence_views = build_sentence_views(&state, job.id, is_done, segments_found).await?;

    let sentence_count = sentence_views.len();
    let template = VideoPageTemplate {
        video_id,
        job_id: job.id,
        video_title: job.video_title,
        status: job.status.as_str().to_string(),
        is_done,
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
        sentence_count,
        sentences: sentence_views,
        segments_found,
        oob_title: false,
    };

    Ok(template.into_response())
}

pub async fn video_status_fragment(
    State(state): State<AppState>,
    Path(video_id): Path<String>,
) -> Result<Response, AppError> {
    let job = db::get_latest_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let is_done = job.status == JobStatus::Done;
    let is_terminal = job.status.is_terminal();
    let segments_found = job.segments_found;
    let sentences = build_sentence_views(&state, job.id, is_done, segments_found).await?;

    let sentence_count = sentences.len();
    let template = VideoContentFragmentTemplate {
        video_id,
        job_id: job.id,
        video_title: job.video_title,
        status: job.status.as_str().to_string(),
        is_done,
        is_terminal,
        error_message: job.error_message,
        sentence_count,
        sentences,
        segments_found,
        oob_title: true,
    };

    Ok(template.into_response())
}

pub async fn export_sentences(
    State(state): State<AppState>,
    HtmlForm(form): HtmlForm<ExportForm>,
) -> Result<Response, AppError> {
    if form.sentence_ids.is_empty() {
        return Ok(ExportErrorTemplate {
            message: "No sentences selected.".into(),
        }
        .into_response());
    }

    let target_word_map = form.target_word_map();
    let sentences = db::get_sentences_by_ids(&state.db, &form.sentence_ids).await?;
    if sentences.is_empty() {
        return Err(AppError::NotFound);
    }

    let job = db::get_job(&state.db, form.job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let source = job
        .video_title
        .unwrap_or_else(|| job.youtube_url.clone());

    // Extract media for each sentence (graceful degradation on failure)
    let mut export_sentences = Vec::with_capacity(sentences.len());
    for sentence in sentences {
        let (screenshot_filename, audio_filename) =
            media_filenames(sentence.job_id, sentence.id);
        let screenshot_path = format!("{}/{screenshot_filename}", state.media_dir);
        let audio_clip_path = format!("{}/{audio_filename}", state.media_dir);

        let mut screenshot_result = None;
        if let Some(video_path) = &job.video_path {
            // Use midpoint of the sentence for the screenshot
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

        let target_word = target_word_map.get(&sentence.id).cloned();

        let (definition, vocab_furigana, vocab_pitch_num) = if let Some(word) = &target_word {
            let result = lookup_word(&state.dictionaries, word).await;
            let furigana = format_furigana(word, &result.reading);
            (result.definition_html, Some(furigana), result.pitch_num)
        } else {
            (None, None, None)
        };

        let mut llm_definition = None;
        if let (Some(word), Some(definer)) = (&target_word, &state.llm_definer) {
            match definer.define(word, &sentence.text).await {
                Ok(def) => llm_definition = Some(def),
                Err(e) => warn!(word, error = %e, "LLM definition failed, exporting without"),
            }
        }

        let sentence_html = target_word.as_ref().and_then(|word| {
            state
                .tokenizer
                .tokenize(&sentence.text)
                .ok()
                .and_then(|tokens| bold_target_in_sentence(&tokens, word))
        });

        export_sentences.push(ExportSentence {
            sentence,
            screenshot_path: screenshot_result,
            audio_clip_path: audio_result,
            target_word,
            definition,
            vocab_furigana,
            vocab_pitch_num,
            sentence_html,
            llm_definition,
        });
    }

    let exported_ids: Vec<i64> = export_sentences.iter().map(|s| s.sentence.id).collect();

    match state
        .exporter
        .export_sentences(export_sentences, source)
        .await
    {
        Ok(count) => Ok(ExportResultTemplate {
            count,
            exported_ids,
        }
        .into_response()),
        Err(e) => {
            let raw = e.to_string();
            warn!(error = %raw, "export to Anki failed");
            let message = if raw.contains("connection")
                || raw.contains("connect")
                || raw.contains("refused")
                || raw.contains("dns")
            {
                "Could not connect to Anki. Is AnkiConnect running?".into()
            } else {
                "Export to Anki failed.".into()
            };
            Ok(ExportErrorTemplate { message }.into_response())
        }
    }
}

pub async fn word_preview(
    State(state): State<AppState>,
    Path((video_id, sentence_id)): Path<(String, i64)>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, AppError> {
    let t0 = std::time::Instant::now();

    let job = db::get_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;
    tracing::debug!(elapsed_ms = t0.elapsed().as_millis(), "get_job_by_video_id");

    let sentences = db::get_sentences_by_ids(&state.db, &[sentence_id]).await?;
    let sentence = sentences.into_iter().next().ok_or(AppError::NotFound)?;
    if sentence.job_id != job.id {
        return Err(AppError::NotFound);
    }
    tracing::debug!(elapsed_ms = t0.elapsed().as_millis(), "get_sentences_by_ids");

    let result = lookup_word(&state.dictionaries, &query.word).await;
    tracing::debug!(elapsed_ms = t0.elapsed().as_millis(), "lookup_word done");

    let response = PreviewFragmentTemplate {
        video_id,
        sentence_id,
        word: query.word,
        reading: result.reading,
        pitch_num: result.pitch_num,
        definition_html: result.definition_html,
    }
    .into_response();
    tracing::debug!(elapsed_ms = t0.elapsed().as_millis(), "word_preview total");

    Ok(response)
}

pub async fn llm_definition(
    State(state): State<AppState>,
    Path((video_id, sentence_id)): Path<(String, i64)>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, AppError> {
    let job = db::get_job_by_video_id(&state.db, &video_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let sentences = db::get_sentences_by_ids(&state.db, &[sentence_id]).await?;
    let sentence = sentences.into_iter().next().ok_or(AppError::NotFound)?;
    if sentence.job_id != job.id {
        return Err(AppError::NotFound);
    }

    let definition = if let Some(definer) = &state.llm_definer {
        match definer.define(&query.word, &sentence.text).await {
            Ok(def) => Some(def),
            Err(e) => {
                warn!(word = query.word, error = %e, "LLM definition failed");
                None
            }
        }
    } else {
        None
    };

    Ok(LlmDefinitionFragmentTemplate { definition }.into_response())
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
            .extract_audio_clip(&audio_path, sentence.start_time, sentence.end_time, &clip_path)
            .await
            .map_err(|e| AppError::Media(e.to_string()))?;
    }

    let bytes = tokio::fs::read(&clip_path)
        .await
        .map_err(|e| AppError::Media(format!("failed to read audio clip: {e}")))?;

    Ok((
        [(axum::http::header::CONTENT_TYPE, "audio/mpeg")],
        bytes,
    )
        .into_response())
}

struct WordLookupResult {
    definition_html: Option<String>,
    reading: String,
    pitch_num: Option<String>,
}

async fn lookup_word(dictionaries: &[Arc<Dictionary>], word: &str) -> WordLookupResult {
    let t0 = std::time::Instant::now();
    let mut def_parts = Vec::new();
    let mut reading = String::new();
    let mut pitch_num = None;

    for (i, dict) in dictionaries.iter().enumerate() {
        let entries = dict.lookup(word).await;
        tracing::debug!(dict = i, elapsed_ms = t0.elapsed().as_millis(), "dict.lookup");
        if let Some(entry) = entries.first() {
            let joined = entry.definitions.join("; ");
            def_parts.push(dict.wrap_definitions(&joined));
            if reading.is_empty() && !entry.reading.is_empty() {
                reading = entry.reading.clone();
            }
        }
        if pitch_num.is_none() {
            let pitch = dict.lookup_pitch(word).await;
            tracing::debug!(dict = i, elapsed_ms = t0.elapsed().as_millis(), "dict.lookup_pitch");
            if !pitch.is_empty() {
                let nums: Vec<String> = pitch[0]
                    .positions
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                pitch_num = Some(nums.join(","));
            }
        }
    }

    WordLookupResult {
        definition_html: if def_parts.is_empty() {
            None
        } else {
            Some(def_parts.join(""))
        },
        reading,
        pitch_num,
    }
}

/// Build sentence HTML with the target word's surface form(s) wrapped in `<b></b>`.
///
/// Matches tokens whose `base_form` equals the target, so conjugated forms are
/// handled correctly (e.g. surface "食べ" with base_form "食べる").
/// Returns `None` if no token matches, so callers can fall back to plain text.
fn bold_target_in_sentence(tokens: &[Token], target_base_form: &str) -> Option<String> {
    if !tokens.iter().any(|t| t.base_form == target_base_form) {
        return None;
    }
    let mut result = String::new();
    for token in tokens {
        if token.base_form == target_base_form {
            result.push_str("<b>");
            result.push_str(&token.surface);
            result.push_str("</b>");
        } else {
            result.push_str(&token.surface);
        }
    }
    Some(result)
}

fn format_seconds(secs: f64) -> String {
    let total = secs as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

#[cfg(test)]
mod tests;
