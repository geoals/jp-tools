use std::collections::HashMap;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use jp_core::tokenize::Tokenizer;

use crate::app::AppState;
use crate::db::{self, VocabUpsert};
use crate::error::AppError;
use crate::models::VocabStatus;

// --- Request/response types ---

#[derive(Deserialize)]
pub struct TokenizeRequest {
    text: String,
}

#[derive(Serialize)]
pub struct TokenizeResponse {
    tokens: Vec<TokenResult>,
}

#[derive(Serialize)]
struct TokenResult {
    lemma: String,
    reading: String,
    pos: String,
    count: i64,
    /// Position of first occurrence in the source text (0-indexed).
    first_occurrence: usize,
    status: Option<VocabStatus>,
    in_db: bool,
    in_dictionary: bool,
}

#[derive(Deserialize)]
pub struct SubmitRequest {
    entries: Vec<SubmitEntry>,
}

#[derive(Deserialize)]
struct SubmitEntry {
    lemma: String,
    reading: String,
    pos: Option<String>,
    status: VocabStatus,
    count: i64,
}

#[derive(Serialize)]
pub struct SubmitResponse {
    count: usize,
}

// --- Helpers ---

/// Verbs and i-adjectives conjugate, producing varying surface readings
/// for the same dictionary word. Group these by base_form only.
fn is_inflecting(pos: &str) -> bool {
    matches!(pos, "動詞" | "形容詞")
}

/// Whether the string contains at least one Japanese character (kanji, hiragana, katakana).
/// Filters out non-Japanese tokens like numbers, Latin text, symbols.
fn contains_japanese(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c,
            '\u{3040}'..='\u{309F}' |  // Hiragana
            '\u{30A0}'..='\u{30FF}' |  // Katakana
            '\u{4E00}'..='\u{9FFF}'    // CJK Unified Ideographs
        )
    })
}

/// Sudachi treats potential forms as independent 下一段 verbs (出せる, 遊べる).
/// Map them back to the godan root (出す, 遊ぶ) for dictionary lookup.
fn godan_root(potential_form: &str) -> Option<String> {
    const MAPPINGS: &[(&str, &str)] = &[
        ("える", "う"),
        ("ける", "く"),
        ("げる", "ぐ"),
        ("せる", "す"),
        ("てる", "つ"),
        ("ねる", "ぬ"),
        ("べる", "ぶ"),
        ("める", "む"),
        ("れる", "る"),
    ];
    for (suffix, root) in MAPPINGS {
        if let Some(stem) = potential_form.strip_suffix(suffix) {
            if !stem.is_empty() {
                return Some(format!("{stem}{root}"));
            }
        }
    }
    None
}

/// Get the dictionary-form reading by tokenizing the base form itself.
fn base_form_reading(tokenizer: &dyn Tokenizer, base_form: &str) -> String {
    tokenizer
        .tokenize(base_form)
        .ok()
        .map(|tokens| tokens.iter().map(|t| t.reading.as_str()).collect())
        .unwrap_or_default()
}

// --- Handlers ---

pub async fn tokenize_text(
    State(state): State<AppState>,
    Json(body): Json<TokenizeRequest>,
) -> Result<Json<TokenizeResponse>, AppError> {
    let text = body.text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest("text is required".into()));
    }

    let all_tokens = state
        .tokenizer
        .tokenize(text)
        .map_err(|e| AppError::BadRequest(format!("tokenization failed: {e}")))?;

    // Filter to content words, deduplicate by (base_form, reading), count occurrences.
    // Track first_occurrence index to preserve original text order.
    let mut counts: HashMap<(String, String), (String, i64, usize)> = HashMap::new();
    let mut base_readings: HashMap<String, String> = HashMap::new();
    let mut next_index: usize = 0;

    for token in &all_tokens {
        if token.pos == "補助記号"
            || !contains_japanese(&token.base_form)
            || token.base_form.contains(|c: char| c.is_ascii_digit())
        {
            continue;
        }

        // Normalize godan potential forms to root verb (出せる → 出す).
        // Only when the form itself isn't a dictionary word — avoids collapsing
        // ichidan/godan pairs like 開ける (ichidan) → 開く (godan).
        let base_form = if token.pos == "動詞"
            && !state.dictionary_forms.contains(&token.base_form)
        {
            godan_root(&token.base_form)
                .filter(|root| state.dictionary_forms.contains(root))
                .unwrap_or_else(|| token.base_form.clone())
        } else {
            token.base_form.clone()
        };

        let reading = if is_inflecting(&token.pos) {
            base_readings
                .entry(base_form.clone())
                .or_insert_with(|| base_form_reading(&*state.tokenizer, &base_form))
                .clone()
        } else {
            token.reading.clone()
        };

        let key = (base_form, reading);
        counts
            .entry(key)
            .and_modify(|(_, c, _)| *c += 1)
            .or_insert_with(|| {
                let idx = next_index;
                next_index += 1;
                (token.pos.clone(), 1, idx)
            });
    }

    // Filter out blacklisted words
    let blacklisted = db::get_blacklisted_keys(&state.db, 1).await?;
    counts.retain(|key, _| !blacklisted.contains(key));

    // Look up existing entries to annotate with DB status
    let lemmas: Vec<String> = counts.keys().map(|(l, _)| l.clone()).collect();
    let existing = db::get_vocab_by_lemmas(&state.db, 1, &lemmas).await?;
    let existing_map: HashMap<(String, String), VocabStatus> = existing
        .into_iter()
        .map(|v| ((v.lemma, v.reading), v.status))
        .collect();

    let mut tokens: Vec<TokenResult> = counts
        .into_iter()
        .map(|((lemma, reading), (pos, count, first_occurrence))| {
            let db_status = existing_map.get(&(lemma.clone(), reading.clone()));
            let in_dictionary = state.dictionary_forms.contains(&lemma);
            TokenResult {
                lemma,
                reading,
                pos,
                count,
                first_occurrence,
                status: db_status.copied(),
                in_db: db_status.is_some(),
                in_dictionary,
            }
        })
        .collect();

    // Default sort: order of first occurrence in source text
    tokens.sort_by_key(|t| t.first_occurrence);

    Ok(Json(TokenizeResponse { tokens }))
}

pub async fn submit_vocab(
    State(state): State<AppState>,
    Json(body): Json<SubmitRequest>,
) -> Result<Json<SubmitResponse>, AppError> {
    if body.entries.is_empty() {
        return Err(AppError::BadRequest("entries is required".into()));
    }

    let upserts: Vec<VocabUpsert> = body
        .entries
        .into_iter()
        .map(|e| VocabUpsert {
            user_id: 1,
            lemma: e.lemma,
            reading: e.reading,
            pos: e.pos,
            status: e.status,
            count: e.count,
            source: "calibration".into(),
        })
        .collect();

    let count = db::upsert_vocab_entries(&state.db, &upserts).await?;
    Ok(Json(SubmitResponse { count }))
}

#[cfg(test)]
mod tests;
