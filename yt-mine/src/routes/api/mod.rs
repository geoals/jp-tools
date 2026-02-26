use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::app::AppState;
use crate::db;
use crate::error::AppError;
use crate::routes::mining::{
    bold_target_in_sentence, build_sentence_views, lookup_word,
};
use crate::services::dictionary::format_furigana;
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
}

#[derive(Deserialize, Default)]
pub struct PollQuery {
    #[serde(default)]
    sc: Option<usize>,
    #[serde(default)]
    st: Option<String>,
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    word: String,
}

#[derive(Serialize)]
struct PreviewResponse {
    word: String,
    reading: String,
    pitch_num: Option<String>,
    definition_html: Option<String>,
}

#[derive(Serialize)]
struct LlmDefinitionResponse {
    definition: Option<String>,
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

pub async fn word_preview(
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

    let result = lookup_word(&state.dictionaries, &query.word).await;

    Ok(Json(PreviewResponse {
        word: query.word,
        reading: result.reading,
        pitch_num: result.pitch_num,
        definition_html: result.definition_html,
    })
    .into_response())
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

    Ok(Json(LlmDefinitionResponse { definition }).into_response())
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
    let target_word_map: std::collections::HashMap<i64, String> = body
        .sentences
        .iter()
        .filter_map(|s| s.target_word.clone().map(|w| (s.id, w)))
        .collect();

    let sentences = db::get_sentences_by_ids(&state.db, &sentence_ids).await?;
    if sentences.is_empty() {
        return Err(AppError::NotFound);
    }

    let job = db::get_job(&state.db, body.job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let source = job
        .video_title
        .unwrap_or_else(|| job.youtube_url.clone());

    let mut export_sentences = Vec::with_capacity(sentences.len());
    for sentence in sentences {
        let (screenshot_filename, audio_filename) =
            media_filenames(sentence.job_id, sentence.id);
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

// --- Helpers ---

fn sentence_to_json(
    view: crate::routes::mining::SentenceView,
) -> SentenceJson {
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
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
