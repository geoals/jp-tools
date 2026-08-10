//! What the reading says about a card that Anki cannot see.
//!
//! Anki schedules on its own review history alone. Meanwhile the same words are
//! being met in the line stream every night, and every one of those encounters
//! is a retrieval it never recorded: a word read without reaching for the
//! dictionary was recalled, and a word looked up despite a long interval was
//! not. This joins the two and sorts each card into a bucket.
//!
//! **Nothing here writes.** The verdict is a reading of the evidence, and the
//! evidence is thin per card — one encounter can be a skimmed line, and the
//! last-review cutoff is Anki's `mod`, which an edit also moves. Bulk action on
//! this without looking at it first is exactly the kind of wrong assertion
//! written at scale that the ledger's two-signal rule exists to prevent.
//!
//! Encounter **days**, not encounters: a word met eight times in one scene is
//! one exposure, and spacing is the whole claim being made.

use std::collections::HashMap;

/// Which of the four readings a card's evidence supports. Ordered by what a
/// sweep should look at first — a mature card being looked up is a hole in the
/// collection, and deferring a card you already know is only ever a saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    /// Looked up despite a long interval. Anki thinks this is known and the
    /// reading says it is not — the one signal Anki structurally cannot have.
    BringForward,
    /// Met on several days since the last review, never looked up.
    Defer,
    /// Long interval, met repeatedly across the reading, never looked up.
    Retire,
    /// Carded, and the reading has not shown it again since.
    NeverMet,
}

/// A card's interval must reach this many days before a lookup on it counts as
/// evidence against Anki. Below it, Anki agrees the word is still being learnt
/// and a lookup tells us nothing new.
pub const MATURE_DAYS: i64 = 21;
/// Distinct days a word must be met on, with no lookup, before its review is
/// treated as one you would have passed.
pub const DEFER_DAYS: i64 = 3;
/// The same, for retiring the card outright, against a long interval.
pub const RETIRE_DAYS: i64 = 6;
pub const RETIRE_INTERVAL: i64 = 60;
/// How old a card must be before "never met since" says anything. A card mined
/// last night has had no chance.
pub const NEVER_MET_AGE_DAYS: f64 = 30.0;

/// One card, its scheduling state, and what the reading has to say about it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CardEvidence {
    pub note_id: i64,
    /// The card's own spelling — what is actually printed on it.
    pub vocab: String,
    /// The ledger key it was resolved to, which is what the counts below were
    /// joined on. Shown because a surprising verdict is usually a surprising
    /// key.
    pub key: String,
    pub interval: i64,
    pub lapses: i64,
    /// Days since the card was last reviewed.
    pub since_review_days: i64,
    /// Distinct days the word was met on since that review.
    pub encounter_days: i64,
    pub encounters: i64,
    pub lookups: i64,
    /// Distinct days met since the card was created, over its whole life.
    pub encounter_days_all: i64,
    pub bucket: Option<Bucket>,
}

/// Everything one call needs, gathered once: the deck, the reading, the
/// lookups. Days are the rollover-adjusted keys `word_days` stores.
pub struct Inputs<'a> {
    /// (note_id, printed spelling, ledger key, interval, lapses, review card,
    /// last review ts, created ts)
    pub cards: &'a [CardInput],
    /// lemma → (day key, count)
    pub word_days: &'a HashMap<String, Vec<(String, i64)>>,
    /// ledger key → lookup timestamps
    pub lookups: &'a HashMap<String, Vec<f64>>,
    pub now: f64,
}

#[derive(Debug, Clone)]
pub struct CardInput {
    pub note_id: i64,
    pub vocab: String,
    pub key: String,
    pub interval: i64,
    pub lapses: i64,
    pub is_review: bool,
    pub last_review_ts: f64,
    /// The note id is its creation time in epoch milliseconds.
    pub created_ts: f64,
    /// The day key of the last review, so the day-keyed encounter rows can be
    /// compared without this module knowing about rollovers or timezones.
    pub last_review_day: String,
    pub created_day: String,
}

pub fn evaluate(input: &Inputs) -> Vec<CardEvidence> {
    input
        .cards
        .iter()
        .map(|c| {
            let days = input.word_days.get(&c.key);
            let (encounter_days, encounters) = count_after(days, &c.last_review_day);
            let (encounter_days_all, _) = count_after(days, &c.created_day);
            let lookups = input
                .lookups
                .get(&c.key)
                .map(|ts| ts.iter().filter(|t| **t > c.last_review_ts).count() as i64)
                .unwrap_or(0);
            let age_days = (input.now - c.created_ts) / 86_400.0;

            let bucket = if !c.is_review {
                // A new or learning card has no claim to test.
                None
            } else if c.interval >= MATURE_DAYS && lookups > 0 {
                Some(Bucket::BringForward)
            } else if lookups == 0
                && encounter_days >= RETIRE_DAYS
                && c.interval >= RETIRE_INTERVAL
            {
                Some(Bucket::Retire)
            } else if lookups == 0 && encounter_days >= DEFER_DAYS {
                Some(Bucket::Defer)
            } else if encounter_days_all == 0 && age_days >= NEVER_MET_AGE_DAYS {
                Some(Bucket::NeverMet)
            } else {
                None
            };

            CardEvidence {
                note_id: c.note_id,
                vocab: c.vocab.clone(),
                key: c.key.clone(),
                interval: c.interval,
                lapses: c.lapses,
                since_review_days: ((input.now - c.last_review_ts) / 86_400.0).round() as i64,
                encounter_days,
                encounters,
                lookups,
                encounter_days_all,
                bucket,
            }
        })
        .collect()
}

/// Encounter days and total encounters strictly after `from` — day keys are
/// ISO, so a string comparison is a date comparison.
fn count_after(days: Option<&Vec<(String, i64)>>, from: &str) -> (i64, i64) {
    let Some(days) = days else { return (0, 0) };
    days.iter()
        .filter(|(day, _)| day.as_str() > from)
        .fold((0, 0), |(d, n), (_, count)| (d + 1, n + count))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86_400.0;

    fn card(interval: i64, last_review_day: &str, is_review: bool) -> CardInput {
        CardInput {
            note_id: 1,
            vocab: "検死".into(),
            key: "検屍".into(),
            interval,
            lapses: 0,
            is_review,
            last_review_ts: 1000.0 * DAY,
            created_ts: 100.0 * DAY,
            last_review_day: last_review_day.into(),
            created_day: "2020-01-01".into(),
        }
    }

    fn inputs<'a>(
        cards: &'a [CardInput],
        word_days: &'a HashMap<String, Vec<(String, i64)>>,
        lookups: &'a HashMap<String, Vec<f64>>,
    ) -> Inputs<'a> {
        Inputs {
            cards,
            word_days,
            lookups,
            now: 1010.0 * DAY,
        }
    }

    fn met(days: &[&str]) -> HashMap<String, Vec<(String, i64)>> {
        HashMap::from([(
            "検屍".to_string(),
            days.iter().map(|d| (d.to_string(), 2)).collect(),
        )])
    }

    #[test]
    fn three_clean_encounter_days_defer() {
        let cards = [card(10, "2026-08-01", true)];
        let days = met(&["2026-08-02", "2026-08-03", "2026-08-04"]);
        let out = evaluate(&inputs(&cards, &days, &HashMap::new()));
        assert_eq!(out[0].bucket, Some(Bucket::Defer));
        assert_eq!(out[0].encounter_days, 3);
        assert_eq!(out[0].encounters, 6);
    }

    #[test]
    fn encounters_before_the_review_do_not_count() {
        let cards = [card(10, "2026-08-05", true)];
        let days = met(&["2026-08-02", "2026-08-03", "2026-08-04"]);
        let out = evaluate(&inputs(&cards, &days, &HashMap::new()));
        assert_eq!(out[0].encounter_days, 0);
        assert_eq!(out[0].bucket, None);
    }

    #[test]
    fn a_lookup_on_a_mature_card_outranks_the_encounters() {
        let cards = [card(90, "2026-08-01", true)];
        let days = met(&["2026-08-02", "2026-08-03", "2026-08-04", "2026-08-05"]);
        let lookups = HashMap::from([("検屍".to_string(), vec![1005.0 * DAY])]);
        let out = evaluate(&inputs(&cards, &days, &lookups));
        assert_eq!(out[0].bucket, Some(Bucket::BringForward));
    }

    #[test]
    fn a_lookup_before_the_review_is_not_evidence_against_it() {
        let cards = [card(90, "2026-08-01", true)];
        let lookups = HashMap::from([("検屍".to_string(), vec![900.0 * DAY])]);
        let out = evaluate(&inputs(&cards, &met(&[]), &lookups));
        assert_eq!(out[0].lookups, 0);
    }

    #[test]
    fn a_long_interval_met_often_retires() {
        let cards = [card(120, "2026-08-01", true)];
        let days = met(&[
            "2026-08-02",
            "2026-08-03",
            "2026-08-04",
            "2026-08-05",
            "2026-08-06",
            "2026-08-07",
        ]);
        let out = evaluate(&inputs(&cards, &days, &HashMap::new()));
        assert_eq!(out[0].bucket, Some(Bucket::Retire));
    }

    #[test]
    fn a_new_card_is_never_bucketed() {
        let cards = [card(0, "2026-08-01", false)];
        let days = met(&["2026-08-02", "2026-08-03", "2026-08-04"]);
        let out = evaluate(&inputs(&cards, &days, &HashMap::new()));
        assert_eq!(out[0].bucket, None);
    }

    #[test]
    fn an_old_card_never_met_says_so() {
        let cards = [card(30, "2026-08-01", true)];
        let out = evaluate(&inputs(&cards, &HashMap::new(), &HashMap::new()));
        assert_eq!(out[0].bucket, Some(Bucket::NeverMet));
    }

    #[test]
    fn a_card_mined_last_week_is_not_never_met() {
        let mut c = card(30, "2026-08-01", true);
        c.created_ts = 1005.0 * DAY;
        let out = evaluate(&inputs(&[c], &HashMap::new(), &HashMap::new()));
        assert_eq!(out[0].bucket, None);
    }
}
