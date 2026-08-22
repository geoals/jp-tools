//! What a word means — the answer a popup draws, assembled out of the
//! dictionary cache.
//!
//! Written for the VN overlay, where Yomitan cannot reach: the overlay draws
//! over the game in fullscreen, no browser can be a layer surface, and
//! QtWebEngine loads no extensions. yt-mine asks the same questions for the
//! same reason — a YouTube transcript has no texthooker to hover a word in — so
//! the assembly lives here and each app keeps only its own route.
//!
//! Segmentation is not repeated here. The caller already has a span per word
//! carrying the `(headword, reading)` the ledger keys on, and passes that pair
//! back, so the popup describes the term the tokenizer decided on rather than
//! the surface under the finger — 振っ is looked up as 振る.
//!
//! [`expand`] is the way out of that when the tokenizer split a word: it scans
//! the raw line to the right of the clicked position, so the reader can pick the
//! compound out of a segmentation nothing downstream could have joined.

use std::sync::Arc;

use serde::Serialize;
use sqlx::SqlitePool;

use crate::highlight::Highlighter;
use crate::knowledge::Knowledge;
use crate::knowledge::dictionaries;
use crate::knowledge::vocabulary::{self, Status, Term};

/// The frequency list that ranks a *spelling with a reading*. Reader-facing
/// ranks come from the reader's frequency dictionary; this one is shown beside
/// it because
/// the two disagree loudly and the difference is informative — 船舶 is 3,843 in
/// newspaper prose against 32,370 in fiction.
const CORPUS_FREQUENCY: &str = "BCCWJ";

/// The dictionary that follows the master in the popup. Named rather than
/// derived: install order is the order zips were first seen, which says nothing
/// about which definition is worth reading first.
pub const SECOND_PAGE: [&str; 1] = ["Jitendex"];

#[derive(Serialize)]
pub struct Sense {
    pub reading: String,
    pub definitions: Vec<String>,
}

#[derive(Serialize)]
pub struct Source {
    pub dictionary: String,
    /// What the popup keys its styling on. Each dictionary carries its own
    /// structure in `data-*` attributes and they collide — `data-headword` and
    /// `data-red` mean one thing in 明鏡 and another in 小学館 — so the rules
    /// have to be scoped to the dictionary they were written for.
    pub slug: String,
    pub senses: Vec<Sense>,
}

#[derive(Serialize)]
pub struct Pitch {
    pub reading: String,
    /// Downstep positions: 0 is heiban, 1 atamadaka, n the mora it falls after.
    pub positions: Vec<u32>,
}

#[derive(Serialize)]
pub struct Definition {
    pub term: String,
    /// Ranked by the reader's frequency dictionary — how common the word is in
    /// fiction.
    pub jiten: Option<i64>,
    /// Ranked by [`CORPUS_FREQUENCY`]. Taken over the spelling alone, so where
    /// a spelling has several readings this is the commonest of them.
    pub bccwj: Option<i64>,
    /// Master dictionary first, then the rest in install order. A dictionary
    /// holding nothing for this term is absent rather than empty.
    pub sources: Vec<Source>,
    /// From whichever installed dictionary carries pitch — NHK here. Narrowed
    /// to the asked-for reading when it lists it, since the accent is a
    /// property of the reading and 空/そら's is not 空/から's.
    pub pitch: Vec<Pitch>,
    /// The lookup row this popup was recorded as, for the client to hand back
    /// if the popup turns out not to have been a lookup.
    ///
    /// [`define`] always leaves this `None` and the caller fills it, because
    /// whether opening a popup is a *lookup* is not a dictionary question: it
    /// is one only while a reading session is running, which is read-stats'
    /// business alone. yt-mine has no session and leaves it unset.
    pub lookup_id: Option<i64>,
}

/// Every definition, pitch and rank the cache holds for one term.
///
/// `reading` narrows the entries where a spelling has several: 空 is そら or
/// から and they are different words. `None` returns every reading.
pub async fn define(
    pool: &SqlitePool,
    term: &str,
    reading: Option<&str>,
) -> Result<Definition, sqlx::Error> {
    let dicts = dictionaries::list_dictionaries(pool).await?;

    let rank_from = async |d: Option<&dictionaries::Dictionary>| match d {
        Some(d) => dictionaries::lookup_frequency(pool, d.id, term)
            .await
            .unwrap_or(None),
        None => None,
    };
    let reader_freq = dicts
        .iter()
        .filter(|d| d.role == dictionaries::Role::Frequency)
        .min_by_key(|d| d.id);

    let mut sources = Vec::new();
    for dict in &dicts {
        let entries = dictionaries::lookup_dictionary_entries(pool, dict.id, term).await?;
        // A frequency or pitch dictionary has no term entries at all, which is
        // what keeps BCCWJ, Jiten and NHK out of the popup without naming them.
        if entries.is_empty() {
            continue;
        }
        // Keep only the asked-for reading when the dictionary actually lists
        // it. When it doesn't, showing every reading beats showing nothing:
        // Sudachi and the dictionary can disagree about how a word is read.
        let matching: Vec<_> = match reading {
            Some(r) if entries.iter().any(|e| e.reading == r) => {
                entries.iter().filter(|e| e.reading == r).collect()
            }
            _ => entries.iter().collect(),
        };
        sources.push(Source {
            dictionary: dict.title.clone(),
            slug: crate::dictionary::css_slug(&dict.title),
            senses: matching
                .into_iter()
                .map(|e| Sense {
                    reading: e.reading.clone(),
                    definitions: e.definitions.clone(),
                })
                .collect(),
        });
    }
    // The master, then Jitendex, then everything else in install order — a
    // stable sort, so the last tier keeps it.
    sources.sort_by_key(|s| {
        if dicts
            .iter()
            .any(|d| d.title == s.dictionary && d.role == dictionaries::Role::Master)
        {
            0
        } else if SECOND_PAGE.contains(&s.dictionary.as_str()) {
            1
        } else {
            2
        }
    });

    let mut pitch = Vec::new();
    for dict in &dicts {
        let entries = dictionaries::lookup_pitch_entries(pool, dict.id, term).await?;
        if entries.is_empty() {
            continue;
        }
        pitch = entries
            .into_iter()
            .filter(|e| reading.is_none_or(|r| e.reading == r))
            .map(|e| Pitch {
                reading: e.reading,
                positions: e.positions,
            })
            .collect();
        break;
    }

    Ok(Definition {
        term: term.to_string(),
        jiten: rank_from(reader_freq).await,
        bccwj: rank_from(dicts.iter().find(|d| d.title == CORPUS_FREQUENCY)).await,
        sources,
        pitch,
        lookup_id: None,
    })
}

/// One `(term, reading)` a dictionary offers for this position — a candidate
/// the popup can be re-opened on, not a match of the text: `term` is the
/// dictionary's spelling and the line may have conjugated it.
#[derive(Serialize)]
pub struct Expansion {
    pub term: String,
    /// The ledger key this spelling resolves to — what judging or mining the
    /// candidate writes, while `term` stays what the dictionary calls it. A
    /// dictionary headword is text from outside the tokenizer: 素振 is
    /// Jitendex's spelling and the ledger's row is 素振り, and writing the raw
    /// string would make a second row that reads as never encountered.
    pub key: String,
    /// One entry per reading, since 素振り is そぶり or すぶり and they are
    /// different words. Empty for a kana headword, which reads as itself.
    pub reading: String,
    /// What the ledger already says about `(key, reading)`. The popup lights
    /// its ✓ or ✗ from this, so re-opening a candidate shows the judgement
    /// made on it last time — without it a term judged a moment ago comes back
    /// looking untouched, which reads as the write having failed.
    pub status: String,
    pub dictionaries: Vec<String>,
}

/// How far to the right a scan reaches, in characters. Yomitan's own default.
const EXPAND_MAX_CHARS: usize = 10;

/// And in tokens, for the deinflected candidates. Five reaches the shape those
/// are for — noun + particle + verb, with room for a second particle.
const EXPAND_MAX_TOKENS: usize = 5;

/// Every other reading of this position, given the line from the clicked word's
/// first character on.
///
/// Two failures, one answer. The tokenizer splits 経年劣化 into 経年 and 劣化,
/// and both halves are right; the compound is a Jitendex headword and not a
/// Sankoku one, so no amount of dictionary validation would have joined them.
/// And where a spelling has several readings it picks one — 素振り as すぶり
/// where the line means そぶり — which is a different word with a different
/// ledger row. Both are the reader seeing what the pipeline cannot, so both get
/// the escape hatch Yomitan gave for free: take the text from the word's first
/// character, and list every `(term, reading)` any dictionary lists for a prefix
/// of it, longest first.
///
/// Candidates come in two shapes, because the two failures do. A literal
/// prefix of the line catches the split compound. A prefix ending on a token
/// boundary with its last token in canonical form
/// ([`Highlighter::prefix_forms`]) catches the conjugated expression —
/// しびれを切らした is listed as しびれを切らす, which no literal prefix of the
/// line spells.
///
/// The highlighter is the caller's own — the one already warm for its line feed
/// — because a second `SudachiTokenizer` is a second pipeline and answers
/// differently, and both halves of this depend on the answer matching the
/// ledger. `None` still works: every candidate then keys on itself, which is
/// what the ledger did before this existed.
pub async fn expand(
    k: &Knowledge,
    highlighter: Option<Arc<Highlighter>>,
    text: &str,
) -> Result<Vec<Expansion>, sqlx::Error> {
    let pool = k.pool();

    let chars: Vec<char> = text.chars().take(EXPAND_MAX_CHARS).collect();
    // A single kanji counts, since it is the shape that most needs a second
    // reading offered: 粋 is いき or すい, both listed, and the ladder picks one.
    // A single kana never does — を, の and た are prefixes of half the lines in
    // the corpus and would put a particle chip in every popup.
    let shortest = match chars.first() {
        Some(c) if crate::text::kanji::is_kanji(*c) => 1,
        _ => 2,
    };
    let mut candidates: Vec<String> = (shortest..=chars.len())
        .map(|n| chars[..n].iter().collect())
        .collect();
    // Two kinds of candidate. A literal prefix finds a compound the tokenizer
    // split; a deinflected one finds an expression the sentence conjugated —
    // しびれを切らした is しびれを切らす in every dictionary that has it, and no
    // prefix of the line spells that.
    if let Some(h) = &highlighter {
        let (h, text) = (h.clone(), text.to_string());
        let forms = tokio::task::spawn_blocking(move || h.prefix_forms(&text, EXPAND_MAX_TOKENS))
            .await
            .unwrap_or_default();
        candidates.extend(forms.into_iter().filter(|f| f.chars().count() > 1));
    }
    // The same word in the other kana alphabet. A line writing アレ means あれ,
    // and only あれ is a headword — アレ is a Jitendex redirect and Sankoku has
    // no entry for it at all, so the popup opened on a cross-reference.
    //
    // Offered here rather than folded by the tokenizer, and that is not
    // timidity: 23 katakana ledger rows would fold onto a master hiragana
    // headword, and 424 of their 549 encounters are ココ, a character in the VN
    // being read. Folding would credit the pronoun ここ with all of them. The
    // reader can tell a name from a word; the pipeline cannot.
    let folded: Vec<String> = candidates
        .iter()
        .map(|c| crate::text::kana::to_hiragana(c))
        .filter(|f| !candidates.contains(f))
        .collect();
    candidates.extend(folded);
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // One statement per dictionary, keyed on `dictionary_id` so it seeks
    // `idx_dictionary_entries_lookup` — there is no index on `term` alone, and
    // a scan of 400k rows here would hold the write lock off every other
    // writer.
    let placeholders = vec!["?"; candidates.len()].join(",");
    let sql = format!(
        "SELECT DISTINCT term, reading FROM dictionary_entries INDEXED BY idx_dictionary_entries_lookup \
         WHERE dictionary_id = ? AND term IN ({placeholders})"
    );

    let mut found: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for dict in dictionaries::list_dictionaries(pool).await? {
        let mut query = sqlx::query_as::<_, (String, String)>(&sql).bind(dict.id);
        for c in &candidates {
            query = query.bind(c);
        }
        for (term, reading) in query.fetch_all(pool).await? {
            let slot = match found.iter_mut().find(|(t, _, _)| t == &term) {
                Some(slot) => slot,
                None => {
                    found.push((term, Vec::new(), Vec::new()));
                    found.last_mut().expect("just pushed")
                }
            };
            if !reading.is_empty() && !slot.1.contains(&reading) {
                slot.1.push(reading);
            }
            if !slot.2.contains(&dict.title) {
                slot.2.push(dict.title.clone());
            }
        }
    }

    found.sort_by_key(|(term, _, _)| std::cmp::Reverse(term.chars().count()));

    // A key resolved by a second pipeline matches no ledger row. With no
    // tokenizer at all, every candidate keys on itself, which is what the
    // ledger did before this existed.
    let keys = match highlighter {
        Some(h) => {
            let terms: Vec<String> = found.iter().map(|(t, _, _)| t.clone()).collect();
            tokio::task::spawn_blocking(move || {
                terms.iter().map(|t| h.ledger_key(t)).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default()
        }
        None => Vec::new(),
    };

    // One candidate per reading, keyed as the ledger keys it.
    let candidates: Vec<(String, String, String, Vec<String>)> = found
        .into_iter()
        .enumerate()
        .flat_map(|(i, (term, readings, dictionaries))| {
            let key = keys.get(i).cloned().unwrap_or_else(|| term.clone());
            let readings = if readings.is_empty() {
                vec![String::new()]
            } else {
                readings
            };
            readings
                .into_iter()
                .map(|reading| (term.clone(), key.clone(), reading, dictionaries.clone()))
                .collect::<Vec<_>>()
        })
        .collect();

    let terms: Vec<Term> = candidates
        .iter()
        .map(|(_, key, reading, _)| Term::new(key.clone(), reading))
        .collect();
    let rows = vocabulary::fetch_many(k, &terms).await?;

    Ok(candidates
        .into_iter()
        .zip(terms)
        .map(|((term, key, reading, dictionaries), t)| Expansion {
            status: rows
                .get(&t)
                .map_or(Status::New, |r| r.status)
                .as_str()
                .to_string(),
            term,
            key,
            reading,
            dictionaries,
        })
        .collect())
}
