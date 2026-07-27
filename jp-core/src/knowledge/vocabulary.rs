//! The vocabulary ledger — what I know, one row per term.
//!
//! Every other table in `knowledge.db` records an event; this one records a
//! state. It is the convergence layer of `spec/knowledge-db.md`'s second axis:
//! the `#read` highlighter, i+1 sentence marking and "how many unknown words
//! are in this video" all reduce to a status lookup here, which is why the
//! counts are stored on the row rather than derived per query.
//!
//! ## Who writes what
//!
//! Three writers, deliberately separated by who owns the fact:
//!
//! | fact | writer | shape |
//! |---|---|---|
//! | encounters | [`record_encounters`], from read-stats' watermarked ingest | incremental |
//! | mined | [`sync_mined`], from the `anki_notes` snapshot | wholesale |
//! | lookups | [`sync_lookup_counts`], from `lookups` | wholesale |
//! | dictionary flags | [`refresh_dictionary_flags`] | wholesale |
//! | **status** | the reader, via [`set_status`] | never by a sync |
//!
//! The wholesale three are recomputed rather than incremented because each
//! mirrors a table that already owns the truth — the same reasoning that makes
//! `anki_notes` a replaced snapshot. Only encounters are incremental, because
//! their source (`lines`) is append-only and re-tokenizing all of it on every
//! Anki refresh would be minutes of CPU for no new information.
//!
//! `status` is touched by none of them. It holds assertions and nothing else,
//! so a resync can never demote a word the reader marked known.

use sqlx::{Row, SqlitePool};

use super::Knowledge;
use crate::text::kana;

/// What the reader has asserted about a term. Never set by a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Ingested from reading, never judged. The default, and distinct from
    /// [`Status::Unknown`] on purpose — see the migration's comment.
    New,
    Known,
    /// Judged, and not known.
    Unknown,
    /// Actively being learned. Set by hand; the Anki sync writes `mined`
    /// instead, so re-syncing cannot demote anything.
    Learning,
    /// Never surface this again.
    Blacklisted,
    /// A proper noun, not vocabulary.
    Name,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::New => "new",
            Status::Known => "known",
            Status::Unknown => "unknown",
            Status::Learning => "learning",
            Status::Blacklisted => "blacklisted",
            Status::Name => "name",
        }
    }

    /// Unrecognized values read back as [`Status::New`] rather than failing:
    /// an unparseable status is an untriaged word, which is the honest answer
    /// and the safe one (it can't silently claim something is known).
    pub fn parse(s: &str) -> Status {
        match s {
            "known" => Status::Known,
            "unknown" => Status::Unknown,
            "learning" => Status::Learning,
            "blacklisted" => Status::Blacklisted,
            "name" => Status::Name,
            _ => Status::New,
        }
    }

    /// Whether this status counts as vocabulary the reader has. Deliberately
    /// *not* the highlighter's rule — that one also weighs `mined`, and gets
    /// to decide for itself (see [`VocabRow::is_known`]).
    pub fn is_known(&self) -> bool {
        matches!(self, Status::Known)
    }

    pub const ALL: [Status; 6] = [
        Status::New,
        Status::Known,
        Status::Unknown,
        Status::Learning,
        Status::Blacklisted,
        Status::Name,
    ];
}

/// A term's identity: the ledger's primary key.
///
/// Constructed through [`Term::new`] rather than built by hand, because the
/// normalization it applies is what makes two writers agree on one row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Term {
    pub headword: String,
    /// Hiragana. Empty when the headword is already kana.
    pub reading: String,
}

impl Term {
    /// Normalize a `(headword, reading)` pair as the tokenizer produced it.
    ///
    /// Two rules, both of which exist so that the same word ingested from a VN
    /// line, a pasted article and an Anki card lands on one row:
    ///
    /// 1. The reading is folded to hiragana. Sudachi emits katakana, the
    ///    dictionaries hold hiragana, and the ledger joins them.
    /// 2. A kana-only headword stores an empty reading. There the two strings
    ///    are the same fact; keeping both would make ください/くださいa
    ///    different row from ください alone, depending on which writer got
    ///    there first.
    pub fn new(headword: impl Into<String>, reading: &str) -> Term {
        let headword = headword.into();
        let reading = if kana::is_all_kana(&headword) {
            String::new()
        } else {
            kana::to_hiragana(reading)
        };
        Term { headword, reading }
    }

    /// The reading to display: the stored one, or the headword when it is kana
    /// and the reading was therefore not stored twice.
    pub fn display_reading(&self) -> &str {
        if self.reading.is_empty() {
            &self.headword
        } else {
            &self.reading
        }
    }
}

/// One ledger row.
#[derive(Debug, Clone)]
pub struct VocabRow {
    pub term: Term,
    pub pos: Option<String>,
    pub status: Status,
    pub status_ts: Option<f64>,
    pub mined: bool,
    pub encounter_count: i64,
    pub lookup_count: i64,
    pub first_seen: Option<f64>,
    pub last_seen: Option<f64>,
    pub in_master: bool,
    pub in_name: bool,
    pub in_reference: bool,
}

impl VocabRow {
    /// The default rule for "the reader has this word": asserted known, or in
    /// Anki. A feature that wants a stricter or looser line (i+1 counting may
    /// want mined-but-new to count as *not* known) should say so itself rather
    /// than change this.
    pub fn is_known(&self) -> bool {
        self.status.is_known() || self.mined
    }

    /// Whether this is a word at all, as opposed to reading noise the
    /// tokenizer produced. Lenient by design: any loaded dictionary having it
    /// is enough. The strict test — [`VocabRow::in_master`] — is for the
    /// vocabulary-size denominator, never for filtering the highlighter.
    pub fn is_word(&self) -> bool {
        self.in_master || self.in_name || self.in_reference
    }
}

fn row_to_vocab(r: &sqlx::sqlite::SqliteRow) -> VocabRow {
    VocabRow {
        term: Term {
            headword: r.get("headword"),
            reading: r.get("reading"),
        },
        pos: r.get("pos"),
        status: Status::parse(r.get("status")),
        status_ts: r.get("status_ts"),
        mined: r.get::<i64, _>("mined") != 0,
        encounter_count: r.get("encounter_count"),
        lookup_count: r.get("lookup_count"),
        first_seen: r.get("first_seen"),
        last_seen: r.get("last_seen"),
        in_master: r.get::<i64, _>("in_master") != 0,
        in_name: r.get::<i64, _>("in_name") != 0,
        in_reference: r.get::<i64, _>("in_reference") != 0,
    }
}

/// One term's contribution from a batch of newly ingested text.
#[derive(Debug, Clone)]
pub struct Encounter {
    pub term: Term,
    pub pos: Option<String>,
    pub count: i64,
    /// Epoch seconds of the earliest and latest occurrence in this batch.
    pub first_ts: f64,
    pub last_ts: f64,
}

/// Add a batch of encounters, creating rows for terms not seen before.
///
/// The only incremental writer. Callers must be watermarked — this adds to
/// `encounter_count` and cannot tell a re-ingest from new reading.
///
/// `status` is untouched on an existing row, including when it is still `new`:
/// encountering a word again says nothing about whether it is known, which is
/// exactly cold-start.md's Pass 4 caveat. Promotion is a decision the reader
/// makes in the triage UI, never something ingest does behind them.
pub async fn record_encounters(k: &Knowledge, batch: &[Encounter]) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = k.pool().begin().await?;
    for e in batch {
        sqlx::query(
            "INSERT INTO vocabulary \
                 (headword, reading, pos, encounter_count, first_seen, last_seen) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(headword, reading) DO UPDATE SET \
                 encounter_count = encounter_count + excluded.encounter_count, \
                 first_seen = MIN(COALESCE(first_seen, excluded.first_seen), excluded.first_seen), \
                 last_seen  = MAX(COALESCE(last_seen,  excluded.last_seen),  excluded.last_seen), \
                 pos = COALESCE(excluded.pos, pos)",
        )
        .bind(&e.term.headword)
        .bind(&e.term.reading)
        .bind(&e.pos)
        .bind(e.count)
        .bind(e.first_ts)
        .bind(e.last_ts)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Mirror `anki_notes` into the `mined` flag, wholesale.
///
/// Matched on headword alone, because that is all Anki has: the VocabKanji
/// field is a dictionary form with no reading beside it. A homograph therefore
/// marks every reading of itself as mined — 辛い mined as からい also shows
/// mined under つらい. That is the honest limit of the source, and it fails in
/// the safe direction for the highlighter (a mined word is not highlighted);
/// the fix, if it ever matters, is a reading on the card, not a guess here.
///
/// Returns how many rows now carry the flag.
pub async fn sync_mined(k: &Knowledge) -> Result<i64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    sqlx::query("UPDATE vocabulary SET mined = 0 WHERE mined = 1")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE vocabulary SET mined = 1 \
         WHERE headword IN (SELECT vocab FROM anki_notes)",
    )
    .execute(&mut *tx)
    .await?;
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vocabulary WHERE mined = 1")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(count.0)
}

/// Recompute `lookup_count` from the `lookups` table, wholesale.
///
/// Also matched on headword alone: Yomitan sends a dictionary form to
/// AnkiConnect and no reading, so a homograph's lookups can't be split between
/// its readings. Recomputed rather than incremented so that discarding lines
/// or fixing the capture guard is reflected on the next refresh instead of
/// leaving a count that can only grow.
pub async fn sync_lookup_counts(k: &Knowledge) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE vocabulary SET lookup_count = \
             (SELECT COUNT(*) FROM lookups WHERE lookups.term = vocabulary.headword)",
    )
    .execute(k.pool())
    .await?;
    Ok(())
}

/// Recompute the three dictionary flags from `dictionary_entries` + the roles.
///
/// A term matches a dictionary if the dictionary lists its headword. The
/// reading is deliberately not required to match: a dictionary that spells a
/// reading differently (送り仮名 variants, an entry with no reading at all)
/// would otherwise make a real word look like tokenizer noise, and the flags
/// exist to answer "is this a word", not "is this exact pair attested".
///
/// **Each subquery must be able to seek `idx_dictionary_entries_lookup`.**
/// That index is `(dictionary_id, term)`, so filtering on `d.role` through a
/// join leaves its leading column unconstrained and SQLite scans all 500k
/// entries *per ledger row*, three times over. Resolving the role to its ids
/// first turns each subquery into a seek on `(dictionary_id = ? AND term = ?)`.
///
/// Not a micro-optimisation: against the real database — 8,276 ledger rows,
/// 518,744 entries — the join form took **six minutes**, and it holds the
/// write lock throughout, so every triage submit and Anki sync during it failed
/// with "database is locked". That is how it was found. The rewrite runs in
/// 15 ms.
pub async fn refresh_dictionary_flags(k: &Knowledge) -> Result<(), sqlx::Error> {
    // Two EXISTS rather than one with an OR inside: each has to be able to
    // seek its own index, and an OR across two columns leaves SQLite scanning.
    //
    // The second is the kana case. A dictionary lists 言う and 出来る in kanji,
    // so a term the tokenizer produced as いう or できる matched nothing and was
    // filed as noise — 398 and 183 encounters of it. When the ledger's reading
    // is empty the headword *is* kana (that is the key's convention), and a
    // dictionary having it as a reading is the same evidence of wordhood that
    // having it as a term would be.
    let clause = |role: &str| {
        let of_role = format!("(SELECT id FROM dictionaries WHERE role = '{role}')");
        format!(
            "(EXISTS (SELECT 1 FROM dictionary_entries de \
                      WHERE de.dictionary_id IN {of_role} \
                        AND de.term = vocabulary.headword) \
              OR (vocabulary.reading = '' AND EXISTS ( \
                      SELECT 1 FROM dictionary_entries de \
                      WHERE de.dictionary_id IN {of_role} \
                        AND de.reading = vocabulary.headword)))"
        )
    };
    sqlx::query(&format!(
        "UPDATE vocabulary SET in_master = {}, in_name = {}, in_reference = {}",
        clause("master"),
        clause("name"),
        clause("reference"),
    ))
    .execute(k.pool())
    .await?;
    Ok(())
}

/// Assert a status on a term, creating the row if the term has never been
/// encountered (the frequency-list triage pass judges words before they have
/// been read).
pub async fn set_status(
    k: &Knowledge,
    term: &Term,
    status: Status,
    ts: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, status_ts) VALUES (?, ?, ?, ?) \
         ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
             status_ts = excluded.status_ts",
    )
    .bind(&term.headword)
    .bind(&term.reading)
    .bind(status.as_str())
    .bind(ts)
    .execute(k.pool())
    .await?;
    Ok(())
}

/// Assert one status across many terms — the bulk half of triage ("mark
/// everything I didn't flag as known"). One transaction, so a batch of ten
/// thousand either lands or doesn't.
pub async fn set_status_bulk(
    k: &Knowledge,
    terms: &[Term],
    status: Status,
    ts: f64,
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    let mut n = 0;
    for term in terms {
        n += sqlx::query(
            "INSERT INTO vocabulary (headword, reading, status, status_ts) VALUES (?, ?, ?, ?) \
             ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
                 status_ts = excluded.status_ts",
        )
        .bind(&term.headword)
        .bind(&term.reading)
        .bind(status.as_str())
        .bind(ts)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

pub async fn fetch(k: &Knowledge, term: &Term) -> Result<Option<VocabRow>, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM vocabulary WHERE headword = ? AND reading = ?")
        .bind(&term.headword)
        .bind(&term.reading)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.as_ref().map(row_to_vocab))
}

/// Every row, most-encountered first — what a triage list is built from.
///
/// Unbounded on purpose at this scale (single-digit thousands of rows); a
/// caller that wants a page can take one. When that stops being true, the
/// filter belongs in SQL, not in a `truncate` here.
pub async fn fetch_all(k: &Knowledge) -> Result<Vec<VocabRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM vocabulary ORDER BY encounter_count DESC, headword")
        .fetch_all(k.pool())
        .await?;
    Ok(rows.iter().map(row_to_vocab).collect())
}

/// The triage queue: untriaged vocabulary, most-encountered first.
///
/// Three filters, each of which is the difference between a queue worth working
/// and one that wastes keystrokes:
///
/// - **`status = 'new'`** — only the never-judged. Re-asking about a word the
///   reader has already ruled on is how a triage pass loses their trust.
/// - **`in_master = 1`** — only master-dictionary terms. The rest are Jitendex
///   phrase headwords, names, and tokenizer noise (`っっ`, `あああ`): they belong
///   in the ledger, but judging them one at a time is pointless, and they are
///   not vocabulary by the definition the scale uses. `spec/knowledge-db.md`.
/// - **`encounter_count >= min_encounters`** — a word met twice is not yet
///   evidence of anything.
///
/// Ordered by encounter count because that is what makes the pass pay: the
/// words met most are the ones every downstream feature will hit most.
pub async fn triage_queue(
    k: &Knowledge,
    min_encounters: i64,
    limit: i64,
) -> Result<Vec<VocabRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT * FROM vocabulary \
         WHERE status = 'new' AND in_master = 1 AND encounter_count >= ? \
         ORDER BY encounter_count DESC, headword LIMIT ?",
    )
    .bind(min_encounters)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(row_to_vocab).collect())
}

/// How many rows the queue would offer at a given threshold, and how many of
/// those the preselect rule would default to `known`.
///
/// Separate from [`triage_queue`] because the UI needs the totals to show what
/// a threshold change does *before* paging through it — a `limit`ed queue
/// cannot answer "how many are left".
pub async fn triage_pending(k: &Knowledge, min_encounters: i64) -> Result<(i64, i64), sqlx::Error> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS total, \
                COALESCE(SUM(CASE WHEN lookup_count = 0 THEN 1 ELSE 0 END), 0) AS preselected \
         FROM vocabulary \
         WHERE status = 'new' AND in_master = 1 AND encounter_count >= ?",
    )
    .bind(min_encounters)
    .fetch_one(k.pool())
    .await?;
    Ok((row.get("total"), row.get("preselected")))
}

/// Whether the triage default would call this term known.
///
/// `encounter_count` alone cannot tell "met 47 times and read straight past it"
/// from "met 47 times and looked up on 12 of them" — and the second is the
/// profile of a word the reader does *not* have. So a single lookup disqualifies
/// the default, whatever the encounter count.
///
/// This is deliberately the same predicate as `spec/cold-start.md`'s Pass 4
/// review query (`status='new' AND encounter_count > n AND lookup_count = 0`),
/// so the ongoing pass is a re-run of this one rather than a second rule that
/// can drift from it.
///
/// It decides a *default*, never a write: the reader submits the judgement.
pub fn preselects_known(row: &VocabRow, min_encounters: i64) -> bool {
    row.encounter_count >= min_encounters && row.lookup_count == 0
}

/// Assert a different status per term, in one transaction.
///
/// [`set_status_bulk`]'s sibling, for the triage submit: a batch is a mix of
/// `known` and `unknown` and has to land atomically, because a partially
/// applied sweep leaves the reader unable to tell which rows they still owe an
/// answer for.
pub async fn set_status_each(
    k: &Knowledge,
    judgements: &[(Term, Status)],
    ts: f64,
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    let mut n = 0;
    for (term, status) in judgements {
        n += sqlx::query(
            "INSERT INTO vocabulary (headword, reading, status, status_ts) VALUES (?, ?, ?, ?) \
             ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
                 status_ts = excluded.status_ts",
        )
        .bind(&term.headword)
        .bind(&term.reading)
        .bind(status.as_str())
        .bind(ts)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

/// Drop rows a rebuild left with nothing behind them.
///
/// After `reset_vocabulary` + a re-ingest, a row that ends on zero encounters
/// is one the current tokenizer no longer produces — a proper noun now that
/// names are excluded, or a term a re-tokenization split differently. Keeping
/// it would leave the ledger's totals counting words that are not in the
/// reading any more.
///
/// Deletes only what nobody has said anything about: never judged (`new`) and
/// not in Anki. A reader's assertion and a mined card both outlive the counts,
/// which is the same rule that lets a rebuild zero the aggregates in the first
/// place.
pub async fn prune_untouched(k: &Knowledge) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM vocabulary \
         WHERE encounter_count = 0 AND status = 'new' AND mined = 0",
    )
    .execute(k.pool())
    .await?
    .rows_affected())
}

/// The rows [`blacklist_non_words`] would hit, commonest first, one page at a
/// time.
///
/// A bulk write the reader cannot see before it lands asks them to trust a
/// predicate they have never been shown. This is that predicate, as a list —
/// the same `WHERE`, and every row of it reachable, because "the top 60"
/// answers whether the head looks like noise and not whether the tail does.
pub async fn non_words(
    k: &Knowledge,
    limit: i64,
    offset: i64,
) -> Result<Vec<VocabRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT * FROM vocabulary \
         WHERE status = 'new' AND in_master = 0 AND in_name = 0 AND in_reference = 0 \
         ORDER BY encounter_count DESC, headword LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(row_to_vocab).collect())
}

/// How many rows the tail holds in total, since the preview is only its head.
pub async fn non_words_total(k: &Knowledge) -> Result<i64, sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vocabulary \
         WHERE status = 'new' AND in_master = 0 AND in_name = 0 AND in_reference = 0",
    )
    .fetch_one(k.pool())
    .await?;
    Ok(n)
}

/// Bulk-`blacklisted` the non-vocabulary tail: rows no dictionary calls a word.
///
/// The counterpart to the queue's `in_master` filter — these are what it
/// excludes, and the only useful action on them is to stop them being offered.
/// The test is the negation of [`VocabRow::is_word`], the lenient one, so this
/// hits only what *nothing* recognizes: tokenizer noise, not obscure vocabulary
/// and not names.
pub async fn blacklist_non_words(k: &Knowledge, ts: f64) -> Result<u64, sqlx::Error> {
    let n = sqlx::query(
        "UPDATE vocabulary SET status = 'blacklisted', status_ts = ? \
         WHERE status = 'new' AND in_master = 0 AND in_name = 0 AND in_reference = 0",
    )
    .bind(ts)
    .execute(k.pool())
    .await?
    .rows_affected();
    Ok(n)
}

/// How many rows sit in each status, and how many of those count toward the
/// vocabulary scale (master-dictionary terms only — see
/// `spec/knowledge-db.md`).
#[derive(Debug, Clone, Default)]
pub struct StatusCount {
    pub status: String,
    pub total: i64,
    pub in_master: i64,
}

pub async fn status_counts(k: &Knowledge) -> Result<Vec<StatusCount>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT status, COUNT(*) AS total, SUM(in_master) AS in_master \
         FROM vocabulary GROUP BY status ORDER BY total DESC",
    )
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| StatusCount {
            status: r.get("status"),
            total: r.get("total"),
            in_master: r.get::<Option<i64>, _>("in_master").unwrap_or(0),
        })
        .collect())
}

/// Whether the ledger has ever been populated. The one thing a caller needs to
/// know before offering to backfill it.
pub async fn is_empty(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vocabulary")
        .fetch_one(pool)
        .await?;
    Ok(count.0 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp() -> Knowledge {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("jp-core-vocab-{nanos}.db"));
        Knowledge::open(path.to_str().unwrap()).await.unwrap()
    }

    fn enc(headword: &str, reading: &str, count: i64, ts: f64) -> Encounter {
        Encounter {
            term: Term::new(headword, reading),
            pos: Some("名詞".into()),
            count,
            first_ts: ts,
            last_ts: ts,
        }
    }

    #[test]
    fn a_reading_is_normalized_to_hiragana() {
        assert_eq!(Term::new("読む", "ヨム").reading, "よむ");
        assert_eq!(Term::new("読む", "よむ").reading, "よむ");
    }

    #[test]
    fn a_kana_headword_does_not_store_its_reading_twice() {
        let t = Term::new("ください", "クダサイ");
        assert_eq!(t.reading, "");
        // ...but it still displays one.
        assert_eq!(t.display_reading(), "ください");
    }

    #[test]
    fn homographs_are_separate_terms() {
        assert_ne!(Term::new("辛い", "カライ"), Term::new("辛い", "ツライ"));
    }

    #[tokio::test]
    async fn encounters_accumulate_and_widen_the_window() {
        let k = temp().await;
        record_encounters(&k, &[enc("読む", "ヨム", 3, 100.0)])
            .await
            .unwrap();
        record_encounters(&k, &[enc("読む", "ヨム", 2, 50.0)])
            .await
            .unwrap();

        let row = fetch(&k, &Term::new("読む", "ヨム"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.encounter_count, 5);
        assert_eq!(row.first_seen, Some(50.0));
        assert_eq!(row.last_seen, Some(100.0));
        // Ingest never judges a word.
        assert_eq!(row.status, Status::New);
    }

    #[tokio::test]
    async fn an_assertion_survives_re_ingest() {
        let k = temp().await;
        let term = Term::new("読む", "ヨム");
        record_encounters(&k, &[enc("読む", "ヨム", 1, 100.0)])
            .await
            .unwrap();
        set_status(&k, &term, Status::Known, 200.0).await.unwrap();
        record_encounters(&k, &[enc("読む", "ヨム", 1, 300.0)])
            .await
            .unwrap();

        let row = fetch(&k, &term).await.unwrap().unwrap();
        assert_eq!(row.status, Status::Known);
        assert_eq!(row.encounter_count, 2);
    }

    #[tokio::test]
    async fn a_status_can_be_asserted_before_the_word_is_ever_read() {
        let k = temp().await;
        let term = Term::new("憂鬱", "ユウウツ");
        set_status(&k, &term, Status::Known, 1.0).await.unwrap();
        let row = fetch(&k, &term).await.unwrap().unwrap();
        assert_eq!(row.status, Status::Known);
        assert_eq!(row.encounter_count, 0);
    }

    #[tokio::test]
    async fn the_anki_sync_sets_mined_without_touching_status() {
        let k = temp().await;
        let term = Term::new("読む", "ヨム");
        record_encounters(&k, &[enc("読む", "ヨム", 1, 1.0)])
            .await
            .unwrap();
        set_status(&k, &term, Status::Known, 2.0).await.unwrap();
        sqlx::query("INSERT INTO anki_notes (note_id, vocab) VALUES (1, '読む')")
            .execute(k.pool())
            .await
            .unwrap();

        assert_eq!(sync_mined(&k).await.unwrap(), 1);
        let row = fetch(&k, &term).await.unwrap().unwrap();
        assert!(row.mined);
        assert_eq!(row.status, Status::Known, "a sync must not write status");

        // A card deleted in Anki clears the flag on the next refresh.
        sqlx::query("DELETE FROM anki_notes")
            .execute(k.pool())
            .await
            .unwrap();
        assert_eq!(sync_mined(&k).await.unwrap(), 0);
        assert!(!fetch(&k, &term).await.unwrap().unwrap().mined);
    }

    #[tokio::test]
    async fn lookup_counts_are_recomputed_not_accumulated() {
        let k = temp().await;
        record_encounters(&k, &[enc("読む", "ヨム", 1, 1.0)])
            .await
            .unwrap();
        for ts in [1.0, 2.0, 3.0] {
            sqlx::query("INSERT INTO lookups (ts, term) VALUES (?, '読む')")
                .bind(ts)
                .execute(k.pool())
                .await
                .unwrap();
        }
        sync_lookup_counts(&k).await.unwrap();
        let term = Term::new("読む", "ヨム");
        assert_eq!(fetch(&k, &term).await.unwrap().unwrap().lookup_count, 3);

        // Running it twice must not double it — that is the point of wholesale.
        sync_lookup_counts(&k).await.unwrap();
        assert_eq!(fetch(&k, &term).await.unwrap().unwrap().lookup_count, 3);
    }

    /// Pins the *plan*, not the result: the flags were correct all along, and
    /// what broke was that each subquery scanned every dictionary entry once
    /// per ledger row. A correctness test cannot see that, and the cost only
    /// shows on a real-sized database.
    #[tokio::test]
    async fn the_flag_refresh_seeks_the_dictionary_index() {
        let k = temp().await;
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN UPDATE vocabulary SET in_master = \
                 (EXISTS (SELECT 1 FROM dictionary_entries de \
                          WHERE de.dictionary_id IN \
                                (SELECT id FROM dictionaries WHERE role = 'master') \
                            AND de.term = vocabulary.headword) \
                  OR (vocabulary.reading = '' AND EXISTS ( \
                          SELECT 1 FROM dictionary_entries de \
                          WHERE de.dictionary_id IN \
                                (SELECT id FROM dictionaries WHERE role = 'master') \
                            AND de.reading = vocabulary.headword)))",
        )
        .fetch_all(k.pool())
        .await
        .unwrap();
        let detail: Vec<&str> = plan.iter().map(|r| r.3.as_str()).collect();
        assert!(
            detail
                .iter()
                .any(|d| d.contains("SEARCH de") && d.contains("idx_dictionary_entries_lookup")),
            "the term subquery must seek the (dictionary_id, term) index: {detail:?}"
        );
        assert!(
            detail
                .iter()
                .any(|d| d.contains("SEARCH de") && d.contains("idx_dictionary_entries_reading")),
            "and the kana one its own (dictionary_id, reading) index: {detail:?}"
        );
        assert!(
            !detail.iter().any(|d| d.starts_with("SCAN de")),
            "scanning every entry per ledger row is the six-minute bug: {detail:?}"
        );
    }

    #[tokio::test]
    async fn pruning_spares_everything_anyone_said_something_about() {
        let k = temp().await;
        let judged = Term::new("鍵", "かぎ");
        let mined = Term::new("扉", "とびら");
        let orphan = Term::new("ノア", "のあ");
        set_status(&k, &judged, Status::Known, 1.0).await.unwrap();
        set_status(&k, &mined, Status::New, 1.0).await.unwrap();
        sqlx::query("UPDATE vocabulary SET mined = 1 WHERE headword = '扉'")
            .execute(k.pool())
            .await
            .unwrap();
        set_status(&k, &orphan, Status::New, 1.0).await.unwrap();

        assert_eq!(prune_untouched(&k).await.unwrap(), 1);
        assert!(fetch(&k, &judged).await.unwrap().is_some(), "judged");
        assert!(fetch(&k, &mined).await.unwrap().is_some(), "in Anki");
        assert!(fetch(&k, &orphan).await.unwrap().is_none(), "nothing left");
    }

    #[tokio::test]
    async fn dictionary_flags_follow_the_role_not_the_dictionary() {
        let k = temp().await;
        record_encounters(
            &k,
            &[enc("読む", "ヨム", 1, 1.0), enc("ああ", "アア", 1, 1.0)],
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '読む', 'よむ', '[]');",
        )
        .execute(k.pool())
        .await
        .unwrap();

        refresh_dictionary_flags(&k).await.unwrap();
        let known = fetch(&k, &Term::new("読む", "ヨム"))
            .await
            .unwrap()
            .unwrap();
        assert!(known.in_master && known.is_word());
        // A token no dictionary has is reading noise, and the gate says so.
        let noise = fetch(&k, &Term::new("ああ", "アア"))
            .await
            .unwrap()
            .unwrap();
        assert!(!noise.is_word());

        // Demote the dictionary: the same term is now a word but not vocabulary.
        super::super::dictionaries::set_role(
            k.pool(),
            1,
            super::super::dictionaries::Role::Reference,
        )
        .await
        .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();
        let after = fetch(&k, &Term::new("読む", "ヨム"))
            .await
            .unwrap()
            .unwrap();
        assert!(!after.in_master);
        assert!(after.in_reference && after.is_word());
    }

    #[tokio::test]
    async fn bulk_triage_counts_toward_the_master_scale_only() {
        let k = temp().await;
        record_encounters(
            &k,
            &[enc("読む", "ヨム", 1, 1.0), enc("ああ見えても", "", 1, 1.0)],
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master'), (2, 'Jitendex', '/j.zip', 'reference');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '読む', 'よむ', '[]'), (2, 'ああ見えても', '', '[]');",
        )
        .execute(k.pool())
        .await
        .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();

        let terms: Vec<Term> = fetch_all(&k)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.term)
            .collect();
        assert_eq!(
            set_status_bulk(&k, &terms, Status::Known, 9.0)
                .await
                .unwrap(),
            2
        );

        let counts = status_counts(&k).await.unwrap();
        let known = counts.iter().find(|c| c.status == "known").unwrap();
        assert_eq!(known.total, 2, "both are known");
        assert_eq!(known.in_master, 1, "only one is a vocabulary word");
    }

    /// The two halves of the preselect rule, and why the lookup half exists.
    #[test]
    fn a_word_ever_looked_up_is_never_preselected_known() {
        let mut row = VocabRow {
            term: Term::new("憂鬱", "ゆううつ"),
            pos: None,
            status: Status::New,
            status_ts: None,
            mined: false,
            encounter_count: 47,
            lookup_count: 0,
            first_seen: None,
            last_seen: None,
            in_master: true,
            in_name: false,
            in_reference: false,
        };
        assert!(preselects_known(&row, 3), "met often, never looked up");

        // One lookup is enough. 47 encounters cannot outvote it: the reader
        // needed help with this word, which is the whole signal.
        row.lookup_count = 1;
        assert!(!preselects_known(&row, 3));

        // And the encounter floor still applies on its own.
        row.lookup_count = 0;
        row.encounter_count = 2;
        assert!(!preselects_known(&row, 3));
    }

    #[tokio::test]
    async fn the_triage_queue_offers_only_unjudged_vocabulary() {
        let k = temp().await;
        record_encounters(
            &k,
            &[
                enc("憂鬱", "ユウウツ", 9, 1.0), // vocabulary, plenty met
                enc("齟齬", "ソゴ", 1, 1.0),     // vocabulary, barely met
                enc("っっ", "", 40, 1.0),        // noise: no dictionary has it
                enc("読む", "ヨム", 20, 1.0),    // vocabulary, but judged below
            ],
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '憂鬱', 'ゆううつ', '[]'), (1, '齟齬', 'そご', '[]'), \
                        (1, '読む', 'よむ', '[]');",
        )
        .execute(k.pool())
        .await
        .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();
        set_status(&k, &Term::new("読む", "よむ"), Status::Known, 2.0)
            .await
            .unwrap();

        let queue = triage_queue(&k, 3, 100).await.unwrap();
        let words: Vec<&str> = queue.iter().map(|r| r.term.headword.as_str()).collect();
        assert_eq!(
            words,
            vec!["憂鬱"],
            "齟齬 is under the floor, っっ is not a word, 読む is already judged"
        );

        // The floor is the only thing keeping 齟齬 out, so lowering it lets it in.
        assert_eq!(triage_queue(&k, 1, 100).await.unwrap().len(), 2);
        // …and っっ stays out at any floor, despite being the most-met row.
        assert!(
            !triage_queue(&k, 1, 100)
                .await
                .unwrap()
                .iter()
                .any(|r| r.term.headword == "っっ")
        );
    }

    #[tokio::test]
    async fn pending_counts_what_the_queue_would_offer_and_tick() {
        let k = temp().await;
        record_encounters(
            &k,
            &[enc("憂鬱", "ユウウツ", 9, 1.0), enc("齟齬", "ソゴ", 9, 1.0)],
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '憂鬱', 'ゆううつ', '[]'), (1, '齟齬', 'そご', '[]');\
             INSERT INTO lookups (ts, term) VALUES (5.0, '齟齬');",
        )
        .execute(k.pool())
        .await
        .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();
        sync_lookup_counts(&k).await.unwrap();

        let (total, preselected) = triage_pending(&k, 3).await.unwrap();
        assert_eq!(total, 2, "both are unjudged vocabulary above the floor");
        assert_eq!(preselected, 1, "only 憂鬱 was never looked up");
    }

    #[tokio::test]
    async fn a_mixed_batch_of_judgements_lands_together() {
        let k = temp().await;
        let known = Term::new("憂鬱", "ゆううつ");
        let unknown = Term::new("齟齬", "そご");
        assert_eq!(
            set_status_each(
                &k,
                &[
                    (known.clone(), Status::Known),
                    (unknown.clone(), Status::Unknown),
                ],
                7.0,
            )
            .await
            .unwrap(),
            2
        );

        assert_eq!(
            fetch(&k, &known).await.unwrap().unwrap().status,
            Status::Known
        );
        let u = fetch(&k, &unknown).await.unwrap().unwrap();
        assert_eq!(u.status, Status::Unknown);
        assert_eq!(u.status_ts, Some(7.0), "an assertion is stamped");
    }

    #[tokio::test]
    async fn the_preview_lists_exactly_what_the_bulk_write_would_hit() {
        let k = temp().await;
        record_encounters(
            &k,
            &[
                enc("っっ", "", 40, 0.0),
                enc("あああ", "", 9, 0.0),
                enc("辞書語", "じしょご", 5, 0.0),
            ],
        )
        .await
        .unwrap();
        sqlx::query("UPDATE vocabulary SET in_reference = 1 WHERE headword = '辞書語'")
            .execute(k.pool())
            .await
            .unwrap();

        let preview = non_words(&k, 50, 0).await.unwrap();
        let listed: Vec<&str> = preview.iter().map(|r| r.term.headword.as_str()).collect();
        assert_eq!(
            listed,
            vec!["っっ", "あああ"],
            "commonest first, no real word"
        );
        assert_eq!(
            blacklist_non_words(&k, 1.0).await.unwrap() as usize,
            preview.len(),
            "the list is the write"
        );
    }

    /// The kana case the wordhood gate used to miss.
    #[tokio::test]
    async fn a_kana_word_the_dictionary_spells_in_kanji_is_still_a_word() {
        let k = temp().await;
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
             VALUES (1, '三省堂', '/tmp/m.zip', 'master')",
        )
        .execute(k.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
             VALUES (1, '言う', 'いう', '[]')",
        )
        .execute(k.pool())
        .await
        .unwrap();

        record_encounters(&k, &[enc("いう", "", 398, 0.0), enc("っっ", "", 40, 0.0)])
            .await
            .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();

        let word = fetch(&k, &Term::new("いう", "")).await.unwrap().unwrap();
        assert!(word.in_master, "the dictionary has it, spelt 言う");
        let noise = fetch(&k, &Term::new("っっ", "")).await.unwrap().unwrap();
        assert!(!noise.is_word(), "and still knows noise from a word");
    }

    #[tokio::test]
    async fn blacklisting_non_words_spares_anything_a_dictionary_knows() {
        let k = temp().await;
        record_encounters(
            &k,
            &[
                enc("あああ", "", 5, 1.0),       // nothing has it
                enc("憂鬱", "ユウウツ", 5, 1.0), // master
                enc("岡部", "オカベ", 5, 1.0),   // a name dictionary has it
            ],
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master'), (2, 'Names', '/n.zip', 'name');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '憂鬱', 'ゆううつ', '[]'), (2, '岡部', 'おかべ', '[]');",
        )
        .execute(k.pool())
        .await
        .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();

        assert_eq!(blacklist_non_words(&k, 8.0).await.unwrap(), 1);
        assert_eq!(
            fetch(&k, &Term::new("あああ", ""))
                .await
                .unwrap()
                .unwrap()
                .status,
            Status::Blacklisted
        );
        // A name is not vocabulary, but it is a word — the queue filters it by
        // in_master, and blacklisting it would be a claim nobody made.
        assert_eq!(
            fetch(&k, &Term::new("岡部", "おかべ"))
                .await
                .unwrap()
                .unwrap()
                .status,
            Status::New
        );
    }
}
