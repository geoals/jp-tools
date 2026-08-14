//! What each work is made of, in words.
//!
//! `word_days` records when a word was met and [`super::vocabulary`] records
//! what is known about it. Neither can say which *work* it was met in, so
//! neither can answer the question a per-work page exists for: how much of this
//! VN do I already know, and which of the words I don't are worth learning
//! before going further into it.
//!
//! Keyed on `(headword, reading)` like the ledger, so the join to it is exact —
//! `word_days`' lemma key would let 辛い borrow the status of its homograph.
//!
//! Two figures, not interchangeable:
//!
//! - **by type** — of the distinct words in this work, how many are known: how
//!   much studying it needs.
//! - **by token** — of its running text, how much is known: how it will *feel*.
//!
//! They routinely sit ten points apart, because the words a work repeats are the
//! common ones.

use sqlx::Row;

use super::Knowledge;
use super::vocabulary::{Status, Term};

/// One term's contribution to one work, from a batch of newly ingested text.
#[derive(Debug, Clone)]
pub struct WorkEncounter {
    pub term: Term,
    pub work: String,
    pub count: i64,
}

/// Add a batch of per-work counts, creating rows for pairs not seen before.
///
/// Additive and not idempotent, exactly like `record_encounters`: the caller
/// must be watermarked, and this sink's watermark moves independently of the
/// other two so a rebuild can re-derive one without double-counting the rest.
pub async fn record_work_terms(k: &Knowledge, batch: &[WorkEncounter]) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = k.pool().begin().await?;
    for e in batch {
        sqlx::query(
            "INSERT INTO work_terms (headword, reading, work, count) VALUES (?, ?, ?, ?) \
             ON CONFLICT(headword, reading, work) DO UPDATE SET \
                 count = count + excluded.count",
        )
        .bind(&e.term.headword)
        .bind(&e.term.reading)
        .bind(&e.work)
        .bind(e.count)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Clear the sink, for the rebuild path. The watermarks are rewound beside it.
pub async fn reset(k: &Knowledge) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM work_terms")
        .execute(k.pool())
        .await?;
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct WorkVocabulary {
    /// Distinct `(headword, reading)` pairs in the work, and total occurrences.
    pub types: i64,
    pub tokens: i64,
    /// Of those, the ones the reader has (asserted known, or mined — the
    /// ledger's own `is_known` rule).
    pub known_types: i64,
    pub known_tokens: i64,
    /// Judged and *not* known: the honest denominator's other half.
    pub unknown_types: i64,
    /// Never judged at all. Kept apart from `unknown_types` because `new` is
    /// not `unknown` — most of a work is normally this, and folding it into
    /// "unknown" would report a coverage figure that is really a triage
    /// backlog.
    pub new_types: i64,
}

impl WorkVocabulary {
    pub fn known_type_pct(&self) -> Option<f64> {
        (self.types > 0).then(|| self.known_types as f64 / self.types as f64 * 100.0)
    }
    pub fn known_token_pct(&self) -> Option<f64> {
        (self.tokens > 0).then(|| self.known_tokens as f64 / self.tokens as f64 * 100.0)
    }
}

/// Only words a dictionary recognizes are counted.
///
/// The tokenizer produces a long tail of fragments that are not words, and
/// including them would put a floor under every "unknown" figure that has
/// nothing to do with the work. This is the same lenient wordhood gate the
/// triage queue uses — any loaded dictionary having it is enough.
pub(crate) const IS_WORD: &str = "(v.in_master = 1 OR v.in_name = 1 OR v.in_reference = 1)";

/// Known: asserted so, mined, **or** known under another reading of the same
/// headword.
///
/// The last clause matches `vocabulary::triage_queue`, which never offers a
/// headword already judged: 皆 marked known as みな leaves 皆/みんな `new`
/// forever, and counting it unknown here would contradict the page next door.
///
/// The cost is a genuine homograph whose readings differ in knownness counting
/// as known throughout — the same trade the queue makes, and making it in one
/// place but not the other would be worse than either choice.
pub(crate) const IS_KNOWN: &str = "(v.status = 'known' OR v.mined = 1 OR EXISTS ( \
     SELECT 1 FROM vocabulary o WHERE o.headword = v.headword AND o.status = 'known'))";

/// The known/unknown split for one work, by type and by token.
pub async fn summary(k: &Knowledge, work: &str) -> Result<WorkVocabulary, sqlx::Error> {
    let known = IS_KNOWN;
    let row = sqlx::query(&format!(
        "SELECT \
             COUNT(*) AS types, \
             COALESCE(SUM(wt.count), 0) AS tokens, \
             COALESCE(SUM({known}), 0) AS known_types, \
             COALESCE(SUM(CASE WHEN {known} THEN wt.count ELSE 0 END), 0) AS known_tokens, \
             COALESCE(SUM(v.status = '{unknown}'), 0) AS unknown_types, \
             COALESCE(SUM(v.status = '{new}'), 0) AS new_types \
         FROM work_terms wt JOIN vocabulary v \
             ON v.headword = wt.headword AND v.reading = wt.reading \
         WHERE wt.work = ? AND {IS_WORD}",
        unknown = Status::Unknown.as_str(),
        new = Status::New.as_str(),
    ))
    .bind(work)
    .fetch_one(k.pool())
    .await?;

    Ok(WorkVocabulary {
        types: row.get("types"),
        tokens: row.get("tokens"),
        known_types: row.get("known_types"),
        known_tokens: row.get("known_tokens"),
        unknown_types: row.get("unknown_types"),
        new_types: row.get("new_types"),
    })
}

#[derive(Debug, Clone)]
pub struct WorkTerm {
    pub headword: String,
    pub reading: String,
    pub pos: Option<String>,
    pub status: String,
    /// Occurrences in this work.
    pub count: i64,
    /// Occurrences everywhere else. The pair is what separates "this work's
    /// vocabulary" from "a word I happen not to know".
    pub elsewhere: i64,
}

fn row_to_term(r: &sqlx::sqlite::SqliteRow) -> WorkTerm {
    WorkTerm {
        headword: r.get("headword"),
        reading: r.get("reading"),
        pos: r.get("pos"),
        status: r.get("status"),
        count: r.get("count"),
        elsewhere: r.get("elsewhere"),
    }
}

/// The unknown words this work uses most — ranked by their frequency **here**,
/// not globally, which is what makes it a reading list for this work rather
/// than a generic frequency deck.
///
/// "Unknown" is everything not known: judged unknown *and* never judged. A
/// learner list that excluded `new` would be nearly empty until triage caught
/// up, and the words a work repeats most are exactly the ones worth judging.
pub async fn top_unknown(
    k: &Knowledge,
    work: &str,
    limit: i64,
) -> Result<Vec<WorkTerm>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT wt.headword, wt.reading, v.pos, v.status, wt.count, \
                (v.encounter_count - wt.count) AS elsewhere \
         FROM work_terms wt JOIN vocabulary v \
             ON v.headword = wt.headword AND v.reading = wt.reading \
         WHERE wt.work = ? AND {IS_WORD} AND NOT {IS_KNOWN} \
           AND v.status IN ('{new}', '{unknown}') \
         ORDER BY wt.count DESC, wt.headword LIMIT ?",
        new = Status::New.as_str(),
        unknown = Status::Unknown.as_str(),
    ))
    .bind(work)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(row_to_term).collect())
}

/// Known words of this work, with the moment each was first met anywhere.
///
/// The raw material for "what this work taught you": the caller decides which
/// of these were first met *here* by asking what was being read at
/// `first_seen`, because only the line stream knows that and this table has no
/// timestamps of its own.
pub async fn known_with_first_seen(
    k: &Knowledge,
    work: &str,
) -> Result<Vec<(WorkTerm, f64)>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT wt.headword, wt.reading, v.pos, v.status, wt.count, \
                (v.encounter_count - wt.count) AS elsewhere, v.first_seen \
         FROM work_terms wt JOIN vocabulary v \
             ON v.headword = wt.headword AND v.reading = wt.reading \
         WHERE wt.work = ? AND {IS_WORD} AND v.first_seen IS NOT NULL AND {IS_KNOWN} \
         ORDER BY wt.count DESC",
    ))
    .bind(work)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| (row_to_term(r), r.get("first_seen")))
        .collect())
}

/// Words this work uses that the rest of the reading barely does.
///
/// The counterweight to [`top_unknown`]: high count here, almost nothing
/// elsewhere. Learning these buys this work and little beyond it, which is
/// worth knowing before spending a week on them.
pub async fn distinctive(
    k: &Knowledge,
    work: &str,
    limit: i64,
) -> Result<Vec<WorkTerm>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT wt.headword, wt.reading, v.pos, v.status, wt.count, \
                (v.encounter_count - wt.count) AS elsewhere \
         FROM work_terms wt JOIN vocabulary v \
             ON v.headword = wt.headword AND v.reading = wt.reading \
         WHERE wt.work = ? AND {IS_WORD} \
           AND wt.count >= 3 AND (v.encounter_count - wt.count) * 4 <= wt.count \
         ORDER BY wt.count DESC, wt.headword LIMIT ?",
    ))
    .bind(work)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(row_to_term).collect())
}

#[cfg(test)]
mod tests {
    use super::super::vocabulary::{Encounter, record_encounters, set_status};
    use super::*;

    async fn temp() -> Knowledge {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jp-core-workterms-{nanos}.db"));
        Knowledge::open(path.to_str().unwrap()).await.unwrap()
    }

    fn we(headword: &str, reading: &str, work: &str, count: i64) -> WorkEncounter {
        WorkEncounter {
            term: Term::new(headword, reading),
            work: work.to_string(),
            count,
        }
    }

    /// Every term needs a ledger row (the join) and a dictionary flag (the
    /// wordhood gate), which is what ingest produces in practice.
    async fn seed(k: &Knowledge, terms: &[(&str, &str, i64)]) {
        let batch: Vec<Encounter> = terms
            .iter()
            .map(|(h, r, n)| Encounter {
                term: Term::new(*h, r),
                pos: Some("名詞".into()),
                count: *n,
                first_ts: 0.0,
                last_ts: 0.0,
            })
            .collect();
        record_encounters(k, &batch).await.unwrap();
        sqlx::query("UPDATE vocabulary SET in_master = 1")
            .execute(k.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn counts_accumulate_per_work() {
        let k = temp().await;
        seed(&k, &[("猫", "ねこ", 5)]).await;
        record_work_terms(&k, &[we("猫", "ねこ", "A", 3)])
            .await
            .unwrap();
        record_work_terms(&k, &[we("猫", "ねこ", "A", 2), we("猫", "ねこ", "B", 7)])
            .await
            .unwrap();
        let a = summary(&k, "A").await.unwrap();
        assert_eq!((a.types, a.tokens), (1, 5));
        let b = summary(&k, "B").await.unwrap();
        assert_eq!((b.types, b.tokens), (1, 7));
    }

    #[tokio::test]
    async fn type_and_token_coverage_are_different_questions() {
        let k = temp().await;
        // One common word read 90 times and known; nine rare ones read once
        // each and never judged.
        seed(&k, &[("人", "ひと", 90)]).await;
        let mut batch = vec![we("人", "ひと", "A", 90)];
        for i in 0..9 {
            let w = format!("語{i}");
            seed(&k, &[(&w, "ご", 1)]).await;
            batch.push(we(&w, "ご", "A", 1));
        }
        record_work_terms(&k, &batch).await.unwrap();
        set_status(&k, &Term::new("人", "ひと"), Status::Known, 0.0)
            .await
            .unwrap();

        let s = summary(&k, "A").await.unwrap();
        assert_eq!((s.types, s.tokens), (10, 99));
        assert_eq!(s.known_types, 1);
        assert_eq!(s.known_tokens, 90);
        // 10% of the vocabulary, 91% of the text — the whole reason both are
        // reported.
        assert_eq!(s.known_type_pct().unwrap().round(), 10.0);
        assert_eq!(s.known_token_pct().unwrap().round(), 91.0);
        assert_eq!(s.new_types, 9, "never judged is not the same as unknown");
        assert_eq!(s.unknown_types, 0);
    }

    #[tokio::test]
    async fn the_unknown_list_ranks_by_frequency_in_this_work() {
        let k = temp().await;
        seed(&k, &[("鍵", "かぎ", 30), ("扉", "とびら", 4)]).await;
        record_work_terms(&k, &[we("鍵", "かぎ", "A", 2), we("扉", "とびら", "A", 4)])
            .await
            .unwrap();
        let list = top_unknown(&k, "A", 10).await.unwrap();
        // 鍵 is commoner overall, 扉 commoner *here* — and here is what a
        // reading list for this work is about.
        assert_eq!(list[0].headword, "扉");
        assert_eq!(list[0].count, 4);
        assert_eq!(list[0].elsewhere, 0);
        assert_eq!(list[1].headword, "鍵");
        assert_eq!(list[1].elsewhere, 28);
    }

    #[tokio::test]
    async fn a_word_known_under_another_reading_counts_as_known() {
        // 皆 was judged as みな; 皆/みんな is never offered again, so counting it
        // unknown would contradict the queue and the reader both.
        let k = temp().await;
        seed(&k, &[("皆", "みな", 235), ("皆", "みんな", 166)]).await;
        record_work_terms(
            &k,
            &[we("皆", "みな", "A", 84), we("皆", "みんな", "A", 151)],
        )
        .await
        .unwrap();
        set_status(&k, &Term::new("皆", "みな"), Status::Known, 1.0)
            .await
            .unwrap();

        let s = summary(&k, "A").await.unwrap();
        assert_eq!(s.known_types, 2, "both readings of a word it knows");
        assert_eq!(s.known_tokens, 235);
        assert!(
            top_unknown(&k, "A", 10).await.unwrap().is_empty(),
            "and neither is offered as a word to learn"
        );
    }

    #[tokio::test]
    async fn a_known_or_mined_word_is_not_in_the_unknown_list() {
        let k = temp().await;
        seed(
            &k,
            &[("猫", "ねこ", 9), ("犬", "いぬ", 9), ("鳥", "とり", 9)],
        )
        .await;
        record_work_terms(
            &k,
            &[
                we("猫", "ねこ", "A", 9),
                we("犬", "いぬ", "A", 9),
                we("鳥", "とり", "A", 9),
            ],
        )
        .await
        .unwrap();
        set_status(&k, &Term::new("猫", "ねこ"), Status::Known, 0.0)
            .await
            .unwrap();
        sqlx::query("UPDATE vocabulary SET mined = 1 WHERE headword = '犬'")
            .execute(k.pool())
            .await
            .unwrap();

        let list = top_unknown(&k, "A", 10).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].headword, "鳥");
    }

    #[tokio::test]
    async fn tokenizer_noise_never_reaches_either_list() {
        let k = temp().await;
        seed(&k, &[("猫", "ねこ", 9)]).await;
        // A fragment no dictionary knows — seeded after the blanket flag
        // update, so it stays unrecognized.
        record_encounters(
            &k,
            &[Encounter {
                term: Term::new("っと", ""),
                pos: None,
                count: 20,
                first_ts: 0.0,
                last_ts: 0.0,
            }],
        )
        .await
        .unwrap();
        record_work_terms(&k, &[we("猫", "ねこ", "A", 9), we("っと", "", "A", 20)])
            .await
            .unwrap();

        let s = summary(&k, "A").await.unwrap();
        assert_eq!(s.types, 1, "only the real word counts");
        assert!(
            top_unknown(&k, "A", 10)
                .await
                .unwrap()
                .iter()
                .all(|t| t.headword != "っと")
        );
    }

    #[tokio::test]
    async fn distinctive_words_are_the_ones_this_work_keeps_to_itself() {
        let k = temp().await;
        seed(&k, &[("結界", "けっかい", 12), ("言う", "いう", 400)]).await;
        record_work_terms(
            &k,
            &[we("結界", "けっかい", "A", 11), we("言う", "いう", "A", 40)],
        )
        .await
        .unwrap();
        let list = distinctive(&k, "A", 10).await.unwrap();
        assert_eq!(list.len(), 1, "言う is everywhere; 結界 is this work's");
        assert_eq!(list[0].headword, "結界");
    }

    #[tokio::test]
    async fn reset_clears_the_sink_and_nothing_else() {
        let k = temp().await;
        seed(&k, &[("猫", "ねこ", 5)]).await;
        record_work_terms(&k, &[we("猫", "ねこ", "A", 5)])
            .await
            .unwrap();
        set_status(&k, &Term::new("猫", "ねこ"), Status::Known, 1.0)
            .await
            .unwrap();

        reset(&k).await.unwrap();
        assert_eq!(summary(&k, "A").await.unwrap().types, 0);
        // The assertion is not a count and must survive a rebuild.
        let row = super::super::vocabulary::fetch(&k, &Term::new("猫", "ねこ"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, Status::Known);
    }
}
