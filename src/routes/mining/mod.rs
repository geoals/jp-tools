use std::sync::Arc;

use axum::extract::{Path, State};
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
use crate::services::dictionary::format_furigana;
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
struct JobPageTemplate {
    job_id: i64,
    video_title: Option<String>,
    status: String,
    is_done: bool,
    is_terminal: bool,
    error_message: Option<String>,
    sentences: Vec<SentenceView>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/job_status_fragment.html")]
struct JobStatusFragmentTemplate {
    job_id: i64,
    status: String,
    is_done: bool,
    is_terminal: bool,
    error_message: Option<String>,
}

#[derive(askama::Template, askama_web::WebTemplate)]
#[template(path = "mining/export_success.html")]
struct ExportSuccessTemplate {
    count: usize,
}

struct SentenceView {
    id: i64,
    timestamp: String,
    tokens: Vec<TokenView>,
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

// --- Handlers ---

pub async fn submit_page() -> impl IntoResponse {
    SubmitTemplate
}

pub async fn submit_youtube(
    State(state): State<AppState>,
    Form(form): Form<SubmitForm>,
) -> Result<Response, AppError> {
    use crate::services::download::is_valid_youtube_url;

    let url = form.url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::BadRequest("URL is required".into()));
    }
    if !is_valid_youtube_url(&url) {
        return Err(AppError::BadRequest("not a valid YouTube URL".into()));
    }

    let job_id = db::create_job(&state.db, &url).await?;

    let pool = state.db.clone();
    let downloader = Arc::clone(&state.downloader);
    let transcriber = Arc::clone(&state.transcriber);
    let audio_dir = state.audio_dir.clone();

    tokio::spawn(async move {
        pipeline::process_job(pool, job_id, url, audio_dir, downloader, transcriber).await;
    });

    Ok(Redirect::to(&format!("/mining/jobs/{job_id}")).into_response())
}

pub async fn job_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let job = db::get_job(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let sentences = if job.status == JobStatus::Done {
        db::get_sentences_for_job(&state.db, id).await?
    } else {
        vec![]
    };

    let sentence_views: Vec<SentenceView> = sentences
        .into_iter()
        .map(|s| {
            let tokens = state
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
                .unwrap_or_default();
            SentenceView {
                id: s.id,
                timestamp: format_seconds(s.start_time),
                tokens,
            }
        })
        .collect();

    let template = JobPageTemplate {
        job_id: job.id,
        video_title: job.video_title,
        status: job.status.as_str().to_string(),
        is_done: job.status == JobStatus::Done,
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
        sentences: sentence_views,
    };

    Ok(template.into_response())
}

pub async fn job_status_fragment(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let job = db::get_job(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    // When terminal, redirect to the full page so htmx replaces the
    // fragment with a full page load showing sentences or error.
    if job.status.is_terminal() {
        return Ok(Redirect::to(&format!("/mining/jobs/{id}")).into_response());
    }

    let template = JobStatusFragmentTemplate {
        job_id: job.id,
        status: job.status.as_str().to_string(),
        is_done: job.status == JobStatus::Done,
        is_terminal: job.status.is_terminal(),
        error_message: job.error_message,
    };

    Ok(template.into_response())
}

pub async fn export_sentences(
    State(state): State<AppState>,
    HtmlForm(form): HtmlForm<ExportForm>,
) -> Result<Response, AppError> {
    if form.sentence_ids.is_empty() {
        return Err(AppError::BadRequest("no sentences selected".into()));
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

        let mut definition = None;
        let mut reading = String::new();
        let mut vocab_pitch_num = None;

        if let Some(word) = &target_word {
            let mut def_parts = Vec::new();
            for dict in &state.dictionaries {
                if let Some(entry) = dict.lookup(word).first() {
                    let joined = entry.definitions.join("; ");
                    def_parts.push(dict.wrap_definitions(&joined));
                    if reading.is_empty() && !entry.reading.is_empty() {
                        reading = entry.reading.clone();
                    }
                }
                if vocab_pitch_num.is_none() {
                    let pitch = dict.lookup_pitch(word);
                    if !pitch.is_empty() {
                        let nums: Vec<String> = pitch[0]
                            .positions
                            .iter()
                            .map(|p| p.to_string())
                            .collect();
                        vocab_pitch_num = Some(nums.join(","));
                    }
                }
            }
            if !def_parts.is_empty() {
                definition = Some(def_parts.join(""));
            }
        }

        let vocab_furigana = target_word
            .as_ref()
            .map(|word| format_furigana(word, &reading));

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
        });
    }

    let count = state
        .exporter
        .export_sentences(export_sentences, source)
        .await
        .map_err(|e| AppError::Export(e.to_string()))?;

    Ok(ExportSuccessTemplate { count }.into_response())
}

pub async fn sentence_audio(
    State(state): State<AppState>,
    Path((job_id, sentence_id)): Path<(i64, i64)>,
) -> Result<Response, AppError> {
    let sentences = db::get_sentences_by_ids(&state.db, &[sentence_id]).await?;
    let sentence = sentences.into_iter().next().ok_or(AppError::NotFound)?;
    if sentence.job_id != job_id {
        return Err(AppError::NotFound);
    }

    let job = db::get_job(&state.db, job_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let audio_path = job
        .audio_path
        .ok_or(AppError::BadRequest("no audio available".into()))?;

    let (_, audio_filename) = media_filenames(job_id, sentence_id);
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
