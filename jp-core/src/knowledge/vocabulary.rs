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
/// Wholesale, and only worth running when a dictionary is imported or a role
/// changes — it is a scan of a 400k-row table against however many terms the
/// ledger holds.
///
/// A term matches a dictionary if the dictionary lists its headword. The
/// reading is deliberately not required to match: a dictionary that spells a
/// reading differently (送り仮名 variants, an entry with no reading at all)
/// would otherwise make a real word look like tokenizer noise, and the flags
/// exist to answer "is this a word", not "is this exact pair attested".
pub async fn refresh_dictionary_flags(k: &Knowledge) -> Result<(), sqlx::Error> {
    let clause = |role: &str| {
        format!(
            "EXISTS (SELECT 1 FROM dictionary_entries de \
                     JOIN dictionaries d ON d.id = de.dictionary_id \
                     WHERE d.role = '{role}' AND de.term = vocabulary.headword)"
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
}
