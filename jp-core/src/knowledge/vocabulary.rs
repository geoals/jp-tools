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

use std::collections::HashMap;

use sqlx::{Row, SqlitePool};

use super::Knowledge;
use crate::text::kana;

/// What counts toward "I know N words", as a SQL predicate.
///
/// One definition, used by every figure that reports a vocabulary size, so
/// they cannot drift apart. The master dictionary's answer, **or** the
/// reader's override where the master has nothing to say: Sankoku carries no
/// 冪等性 and no 冪等 either, and admitting JMdict instead would drag in every
/// idiom and orthographic variant. See the migration note on `promoted`.
pub const COUNTS_AS_VOCAB: &str = "(in_master = 1 OR promoted = 1)";

/// What the reader has asserted about a term. Never set by a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Ingested from reading, never judged. The default, and distinct from
    /// [`Status::Unknown`] on purpose — see the migration's comment.
    New,
    Known,
    /// Judged, and not known.
    ///
    /// The sweep's snooze. Without it a word met often but not known comes
    /// back in every periodic batch forever, so this is what "no" writes —
    /// not a state anyone sets deliberately.
    Unknown,
    /// Never surface this again.
    Blacklisted,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::New => "new",
            Status::Known => "known",
            Status::Unknown => "unknown",
            Status::Blacklisted => "blacklisted",
        }
    }

    /// Unrecognized values read back as [`Status::New`] rather than failing:
    /// an unparseable status is an untriaged word, which is the honest answer
    /// and the safe one (it can't silently claim something is known).
    pub fn parse(s: &str) -> Status {
        match s {
            "known" => Status::Known,
            "unknown" => Status::Unknown,
            "blacklisted" => Status::Blacklisted,
            _ => Status::New,
        }
    }

    /// Whether this status counts as vocabulary the reader has. Deliberately
    /// *not* the highlighter's rule — that one also weighs `mined`, and gets
    /// to decide for itself (see [`VocabRow::is_known`]).
    pub fn is_known(&self) -> bool {
        matches!(self, Status::Known)
    }

    /// `learning` and `name` were removed in 2026-07: both carried zero rows.
    /// `learning` duplicated `mined`, which the Anki sync already maintains
    /// beside `status`; a name is kept out of the ledger at ingest by Sudachi's
    /// 固有名詞 subclass, so a status for one had nothing to mark. Anything
    /// still spelling them in the database reads back as `new` via [`parse`],
    /// which is the safe answer.
    pub const ALL: [Status; 4] = [
        Status::New,
        Status::Known,
        Status::Unknown,
        Status::Blacklisted,
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
        "INSERT INTO vocabulary (headword, reading, status, status_ts, status_source) \
             VALUES (?, ?, ?, ?, 'assert') \
         ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
             status_ts = excluded.status_ts, status_source = excluded.status_source",
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
            "INSERT INTO vocabulary (headword, reading, status, status_ts, status_source) \
                 VALUES (?, ?, ?, ?, 'bulk') \
             ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
                 status_ts = excluded.status_ts, status_source = excluded.status_source",
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

/// One row of a frequency-triage page: a term, its BCCWJ rank, and its single
/// resolved master-dictionary reading. Always resolvable — [`frequency_queue`]
/// only returns rows with exactly one reading; a homograph never reaches this
/// struct at all; see [`frequency_pending`]'s `ambiguous` count for those.
#[derive(Debug, Clone)]
pub struct FrequencyCandidate {
    pub term: String,
    pub rank: i64,
    pub reading: String,
}

/// The shared filter behind both frequency-triage functions: BCCWJ rows within
/// `dictionary_id`, restricted to master-dictionary terms (the same rule the
/// triage queue uses, so a frequency word not in the master scale can't be
/// marked known here either), excluding any headword already carrying a
/// non-`new` status under *any* of its readings — the same "don't ask about a
/// word twice" rule `UNJUDGED_HEADWORD` applies, but simpler here since no
/// ledger row need exist yet for a term this pass has never seen.
///
/// Both EXISTS clauses seek `idx_dictionary_entries_lookup`/
/// `idx_vocabulary_headword` rather than scanning — see the perf note on
/// [`refresh_dictionary_flags`] for why that distinction matters here too.
const FREQUENCY_FILTER: &str = "df.dictionary_id = ? \
     AND EXISTS (SELECT 1 FROM dictionary_entries de \
                 WHERE de.dictionary_id = ? AND de.term = df.term) \
     AND NOT EXISTS (SELECT 1 FROM vocabulary v \
                     WHERE v.headword = df.term AND v.status != 'new')";

/// How many pending terms at a threshold a commit would actually write, and
/// how many it would skip as homographs.
///
/// Kept as two numbers rather than one "pending" count because a term with no
/// ledger row yet stays pending forever once it's a homograph — nothing here
/// ever resolves one, so an ambiguous term reappears at every future preview
/// or commit at the same threshold. A single count conflated the two and let
/// a "mark all N known" button promise work a second commit could not do: hit
/// once, everything committable is gone, and what's left is only the
/// standing homograph tail — the same N showing up again looks like the
/// button did nothing, when what happened is it did exactly what it could.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrequencyPending {
    pub committable: i64,
    pub ambiguous: i64,
}

/// How many BCCWJ terms at or under `max_rank` are still unjudged master
/// vocabulary — the count a threshold slider shows before a preview is asked
/// for.
///
/// **The reading-count subquery is pinned to `idx_dictionary_entries_lookup`
/// with `INDEXED BY`, and must stay pinned.** `COUNT(DISTINCT de.reading)`
/// gives the planner a reason to prefer `idx_dictionary_entries_reading`
/// instead — that index is keyed `(dictionary_id, reading)`, so it can only
/// seek on `dictionary_id`, and the `term = df.term` filter becomes a scan of
/// every master-dictionary entry for that dictionary, once per BCCWJ row this
/// pass considers. Caught here rather than shipped: `EXPLAIN QUERY PLAN`
/// showed `SEARCH de USING INDEX idx_dictionary_entries_reading
/// (dictionary_id=?)` — no `term` bound at all — and the query itself hung
/// past a 15s timeout at a 6,000-word threshold against the real database.
/// Forcing the lookup index turned that into 0.6s. Exactly the class of bug
/// [`refresh_dictionary_flags`]'s doc comment warns about: check `EXPLAIN
/// QUERY PLAN` says SEARCH with both key columns bound, not a bare
/// `dictionary_id` seek standing in for one.
pub async fn frequency_pending(
    k: &Knowledge,
    bccwj_id: i64,
    master_id: i64,
    max_rank: i64,
) -> Result<FrequencyPending, sqlx::Error> {
    let row = sqlx::query(&format!(
        "SELECT \
             COALESCE(SUM(CASE WHEN readings <= 1 THEN 1 ELSE 0 END), 0) AS committable, \
             COALESCE(SUM(CASE WHEN readings > 1 THEN 1 ELSE 0 END), 0) AS ambiguous \
         FROM ( \
             SELECT df.term, \
                    (SELECT COUNT(DISTINCT de.reading) FROM dictionary_entries de \
                     INDEXED BY idx_dictionary_entries_lookup \
                     WHERE de.dictionary_id = ? AND de.term = df.term) AS readings \
             FROM dictionary_frequency df \
             WHERE {FREQUENCY_FILTER} \
             GROUP BY df.term HAVING MIN(df.frequency) <= ?)"
    ))
    .bind(master_id)
    .bind(bccwj_id)
    .bind(master_id)
    .bind(max_rank)
    .fetch_one(k.pool())
    .await?;
    Ok(FrequencyPending {
        committable: row.get("committable"),
        ambiguous: row.get("ambiguous"),
    })
}

/// A page of the frequency-triage candidates, commonest (lowest rank) first —
/// **committable ones only**. A homograph is excluded here, not merely
/// flagged, for two reasons: nothing this pass can do with it (it cannot be
/// written without guessing a reading), and a page is worth exactly as much
/// as the rows on it can be acted on — spending page slots on rows the reader
/// has to click past for no gain is the friction this pass exists to remove.
/// [`frequency_pending`]'s `ambiguous` count is where that word is still
/// visible, as a total rather than a name.
///
/// Both extra subqueries resolve the reading count and the reading itself
/// through `idx_dictionary_entries_lookup`, `INDEXED BY`-pinned for the same
/// reason [`frequency_pending`]'s is: `COUNT`/`MIN` over `reading` gives the
/// planner a reason to prefer `idx_dictionary_entries_reading`, which cannot
/// seek on `term` and turns each row into a scan of the whole master
/// dictionary. That regression is exactly what sent a next-page click into
/// several extra seconds before this was pinned — on top of what committing a
/// reading per row in Rust, one round trip at a time, already cost.
pub async fn frequency_queue(
    k: &Knowledge,
    bccwj_id: i64,
    master_id: i64,
    max_rank: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<FrequencyCandidate>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT term, rank, reading FROM ( \
             SELECT df.term AS term, MIN(df.frequency) AS rank, \
                    (SELECT COUNT(DISTINCT de.reading) FROM dictionary_entries de \
                     INDEXED BY idx_dictionary_entries_lookup \
                     WHERE de.dictionary_id = ? AND de.term = df.term) AS reading_count, \
                    (SELECT MIN(de.reading) FROM dictionary_entries de \
                     INDEXED BY idx_dictionary_entries_lookup \
                     WHERE de.dictionary_id = ? AND de.term = df.term) AS reading \
             FROM dictionary_frequency df \
             WHERE {FREQUENCY_FILTER} \
             GROUP BY df.term HAVING MIN(df.frequency) <= ? AND reading_count = 1) \
         ORDER BY rank, term LIMIT ? OFFSET ?"
    ))
    .bind(master_id)
    .bind(master_id)
    .bind(bccwj_id)
    .bind(master_id)
    .bind(max_rank)
    .bind(limit)
    .bind(offset)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| FrequencyCandidate {
            term: r.get("term"),
            rank: r.get("rank"),
            reading: r.get("reading"),
        })
        .collect())
}

pub async fn fetch(k: &Knowledge, term: &Term) -> Result<Option<VocabRow>, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM vocabulary WHERE headword = ? AND reading = ?")
        .bind(&term.headword)
        .bind(&term.reading)
        .fetch_optional(k.pool())
        .await?;
    Ok(row.as_ref().map(row_to_vocab))
}

/// The rows for a handful of terms at once, keyed by term.
///
/// For the reader's highlighter, which asks about the dozen or so words in one
/// hooked line and needs the answer between the line arriving and it being
/// drawn. One query rather than a [`fetch`] per token: the round trips, not the
/// index seeks, are what would show up beside a 30ms poll.
///
/// Terms with no row are simply absent from the map — a word nobody has
/// ingested yet is not an error, it is the ordinary state of a line that was
/// hooked a second ago.
pub async fn fetch_many(
    k: &Knowledge,
    terms: &[Term],
) -> Result<HashMap<Term, VocabRow>, sqlx::Error> {
    if terms.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["(?, ?)"; terms.len()].join(", ");
    let sql = format!("SELECT * FROM vocabulary WHERE (headword, reading) IN ({placeholders})");
    let mut q = sqlx::query(&sql);
    for t in terms {
        q = q.bind(&t.headword).bind(&t.reading);
    }
    let rows = q.fetch_all(k.pool()).await?;
    Ok(rows
        .iter()
        .map(row_to_vocab)
        .map(|r| (r.term.clone(), r))
        .collect())
}

/// Which of these headwords are known under *any* of their readings, and under
/// which one.
///
/// The lookup behind "a word judged under one reading is not judged again" —
/// `UNJUDGED_HEADWORD`'s rule and `work_terms::IS_KNOWN`'s, for a caller
/// holding terms rather than writing SQL. The ledger keys on
/// `(headword, reading)` for the homograph case, but most pairs it produces are
/// one word the dictionary lists twice, and Sudachi's reading for an inflected
/// form is a third way to land on a second row: 通れ resolves to the headword
/// 通る with the reading とおれる, which is a row of its own beside 通る/とおる.
/// Anything that shows the reader a *word* should treat them as one.
///
/// The reading comes back with it because a caller that lets the reader *act*
/// on the word needs to know which row carries the assertion: taking 通る back
/// has to write to 通る/とおる, the row that says known, not to the 通る/とおれる
/// the tokenizer happened to produce — which would leave the word looking
/// exactly as it did and the ledger still claiming it is known. Readings are
/// ordered so the answer is stable when a headword is known under two.
///
/// Deliberately not [`fetch_many`] with a looser key: this asks a different
/// question ("is this word known at all") and a caller wanting the exact row's
/// status still gets it from there.
pub async fn known_readings(
    k: &Knowledge,
    headwords: &[String],
) -> Result<HashMap<String, String>, sqlx::Error> {
    if headwords.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; headwords.len()].join(", ");
    let sql = format!(
        "SELECT headword, reading FROM vocabulary \
         WHERE headword IN ({placeholders}) AND (status = 'known' OR mined = 1) \
         ORDER BY reading DESC"
    );
    let mut q = sqlx::query(&sql);
    for h in headwords {
        q = q.bind(h);
    }
    // DESC, then collect: the last row for a headword wins, so this keeps the
    // first reading in order rather than an arbitrary one.
    Ok(q.fetch_all(k.pool())
        .await?
        .iter()
        .map(|r| (r.get("headword"), r.get("reading")))
        .collect())
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
/// - **[`COUNTS_AS_VOCAB`]** — only master-dictionary terms, plus the ones the
///   reader promoted where the master has nothing to say. The rest are Jitendex
///   phrase headwords, names, and tokenizer noise (`っっ`, `あああ`): they belong
///   in the ledger, but judging them one at a time is pointless, and they are
///   not vocabulary by the definition the scale uses. `spec/knowledge-db.md`.
/// - **`encounter_count >= min_encounters`** — a word met twice is not yet
///   evidence of anything.
///
/// Ordered by encounter count because that is what makes the pass pay: the
/// words met most are the ones every downstream feature will hit most.
/// A word the reader has already judged under one of its readings is not
/// offered again under another.
///
/// The ledger keys on `(headword, reading)` because 空 is そら or から and
/// judging one must not judge the other. But most of the pairs that produces
/// are not homographs — they are one word the dictionary lists twice (言う as
/// いう and ゆう, 皆 as みな and みんな), and offering the second after the
/// reader has judged the first asks them to rule on the same word again.
///
/// This only stops the asking. It writes nothing: the second row stays `new`,
/// which is true — nobody judged *it* — and a reader who wants it judged can
/// still reach it. Inheriting the status instead would be a sync writing
/// `status`, which nothing here is allowed to do.
const UNJUDGED_HEADWORD: &str = "NOT EXISTS (SELECT 1 FROM vocabulary o \
     WHERE o.headword = vocabulary.headword AND o.reading != vocabulary.reading \
       AND o.status != 'new')";

/// Scope a queue to what the reading has touched since the last sweep.
///
/// The periodic sweep asks "what became ready since I last looked", and
/// `last_seen` is the only column that can answer anything like it. It answers
/// a slightly wider question — *met* since the mark, not *crossed the
/// threshold* since the mark — so a word declined last time comes back the
/// next time it is read rather than staying gone until it is read a lot more.
/// `word_days` could answer it exactly, at the cost of a per-term aggregate on
/// every load; that trade is only worth making if the batches come out noisy
/// (`spec/periodic-sweep.md`).
const MET_SINCE: &str = "AND COALESCE(last_seen, 0) > ?";

/// The queue's filter, shared by [`triage_queue`] and [`triage_pending`] so a
/// count on screen cannot promise rows the queue will not offer.
fn queue_where(since_ts: Option<f64>) -> String {
    let met_since = if since_ts.is_some() { MET_SINCE } else { "" };
    format!(
        "status = 'new' AND {COUNTS_AS_VOCAB} AND encounter_count >= ? \
         {met_since} AND {UNJUDGED_HEADWORD}"
    )
}

pub async fn triage_queue(
    k: &Knowledge,
    min_encounters: i64,
    since_ts: Option<f64>,
    limit: i64,
) -> Result<Vec<VocabRow>, sqlx::Error> {
    let sql = format!(
        "SELECT * FROM vocabulary WHERE {} \
         ORDER BY encounter_count DESC, headword LIMIT ?",
        queue_where(since_ts)
    );
    let mut q = sqlx::query(&sql).bind(min_encounters);
    if let Some(ts) = since_ts {
        q = q.bind(ts);
    }
    let rows = q.bind(limit).fetch_all(k.pool()).await?;
    Ok(rows.iter().map(row_to_vocab).collect())
}

/// How many rows the queue would offer at a given threshold, and how many of
/// those the preselect rule would default to `known`.
///
/// Separate from [`triage_queue`] because the UI needs the totals to show what
/// a threshold change does *before* paging through it — a `limit`ed queue
/// cannot answer "how many are left". `since_ts` must match the queue's, or
/// the header would count a different batch from the one on screen.
pub async fn triage_pending(
    k: &Knowledge,
    min_encounters: i64,
    since_ts: Option<f64>,
) -> Result<(i64, i64), sqlx::Error> {
    let sql = format!(
        "SELECT COUNT(*) AS total, \
                COALESCE(SUM(CASE WHEN lookup_count = 0 THEN 1 ELSE 0 END), 0) AS preselected \
         FROM vocabulary WHERE {}",
        queue_where(since_ts)
    );
    let mut q = sqlx::query(&sql).bind(min_encounters);
    if let Some(ts) = since_ts {
        q = q.bind(ts);
    }
    let row = q.fetch_one(k.pool()).await?;
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
/// Assert `status` only where nothing has been asserted yet.
///
/// The write a *bulk seed* wants, as against [`set_status_each`]'s write,
/// which a person clicking a row wants. A seed carries thousands of
/// judgements made somewhere else and months ago; a row already saying
/// `unknown` carries one made here, with the word on screen. The seed must
/// not overrule it — the first jiten import quietly turned 16 `unknown` rows
/// into `known`, which is exactly the demotion-in-reverse that
/// "only the reader writes status" exists to prevent.
///
/// So an existing row is written only when it is still `new`. Rows that do
/// not exist are created. That also makes the import idempotent and
/// order-independent: running it twice, or after any other pass, cannot
/// change an answer that is already there.
pub async fn seed_status_each(
    k: &Knowledge,
    judgements: &[(Term, Status)],
    ts: f64,
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    let mut n = 0;
    for (term, status) in judgements {
        n += sqlx::query(
            "INSERT INTO vocabulary (headword, reading, status, status_ts, status_source) \
                 VALUES (?, ?, ?, ?, 'seed') \
             ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
                 status_ts = excluded.status_ts, status_source = excluded.status_source \
             WHERE vocabulary.status = 'new'",
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

/// `source` labels the pass in the history log — `triage`, `jiten`,
/// `frequency`. It is informational; it never affects what is written to
/// `vocabulary` itself.
pub async fn set_status_each(
    k: &Knowledge,
    judgements: &[(Term, Status)],
    ts: f64,
    source: &str,
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    let mut n = 0;
    for (term, status) in judgements {
        n += sqlx::query(
            "INSERT INTO vocabulary (headword, reading, status, status_ts, status_source) \
                 VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(headword, reading) DO UPDATE SET status = excluded.status, \
                 status_ts = excluded.status_ts, status_source = excluded.status_source",
        )
        .bind(&term.headword)
        .bind(&term.reading)
        .bind(status.as_str())
        .bind(ts)
        .bind(source)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

/// Move a judgement from a key the ingest no longer produces onto the one it
/// does, and delete the old row.
///
/// Re-tokenization strands assertions. When the ledger began keying on
/// Sudachi's *normalized* form, いっぱい and あげる became 一杯 and 上げる, and
/// 209 words the reader had marked known were left on rows nothing writes to
/// any more — present, correct, and attached to a spelling the ingest has
/// stopped producing.
///
/// Only the judgement moves: the counts on a stranded row are zero by
/// definition (the rebuild recomputed them onto the new key). A target that
/// already carries its own assertion keeps it — this repairs a key change, it
/// does not arbitrate between two things the reader said.
///
/// Returns whether anything moved.
pub async fn carry_judgement(k: &Knowledge, from: &Term, into: &Term) -> Result<bool, sqlx::Error> {
    if from == into {
        return Ok(false);
    }
    let mut tx = k.pool().begin().await?;
    let moved = sqlx::query(
        "UPDATE vocabulary SET status = (SELECT status FROM vocabulary \
                                         WHERE headword = ? AND reading = ?), \
                               status_ts = (SELECT status_ts FROM vocabulary \
                                            WHERE headword = ? AND reading = ?), \
                               status_source = 'carry' \
         WHERE headword = ? AND reading = ? AND status = 'new' \
           AND EXISTS (SELECT 1 FROM vocabulary \
                       WHERE headword = ? AND reading = ? AND status != 'new')",
    )
    .bind(&from.headword)
    .bind(&from.reading)
    .bind(&from.headword)
    .bind(&from.reading)
    .bind(&into.headword)
    .bind(&into.reading)
    .bind(&from.headword)
    .bind(&from.reading)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    // Only once it landed somewhere. A stranded judgement whose word is not in
    // the ledger at all — no longer read, or renamed to something the
    // tokenizer will not confirm — is kept: it is the reader's, it costs a
    // row, and deleting it would be losing an answer to save space.
    if moved > 0 {
        sqlx::query(
            "DELETE FROM vocabulary WHERE headword = ? AND reading = ? AND encounter_count = 0",
        )
        .bind(&from.headword)
        .bind(&from.reading)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(moved > 0)
}

/// Every judged row the last ingest left with nothing behind it — the input to
/// [`carry_judgement`], since only the caller has a tokenizer to say what each
/// one is called now.
pub async fn stranded_judgements(k: &Knowledge) -> Result<Vec<VocabRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT * FROM vocabulary WHERE status != 'new' AND encounter_count = 0 AND mined = 0",
    )
    .fetch_all(k.pool())
    .await?;
    Ok(rows.iter().map(row_to_vocab).collect())
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
        "UPDATE vocabulary SET status = 'blacklisted', status_ts = ?, \
                               status_source = 'blacklist' \
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
    let rows = sqlx::query(&format!(
        "SELECT status, COUNT(*) AS total, \
                SUM(CASE WHEN {COUNTS_AS_VOCAB} THEN 1 ELSE 0 END) AS in_master \
         FROM vocabulary GROUP BY status ORDER BY total DESC"
    ))
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

/// A term that would count toward the vocabulary scale if the reader said so.
///
/// The escape hatch's queue. Only two things ever put a term here, and both
/// are evidence the reader cares about it:
///
/// - **it was mined** — a card exists, which is the reader claiming the word.
///   This is the main source, and structurally the *only* way a non-master
///   term can reach them at all: the triage queue and the periodic sweep both
///   gate on the vocabulary predicate, so a word Sankoku does not list can
///   never be offered by reading-based triage.
/// - **it was read often** — 懲罰房 was met 61 times and credited to nothing,
///   because Sudachi tags it a place name and so it is never decomposed into
///   parts the master dictionary does list.
///
/// Deliberately not "every non-master term": most of those are tokenizer
/// noise, and the blacklist bulk action exists for them.
#[derive(Debug, Clone)]
pub struct PromotionCandidate {
    pub term: Term,
    pub mined: bool,
    pub encounter_count: i64,
    pub in_reference: bool,
}

/// Terms outside the vocabulary scale that are worth a promote/skip decision.
///
/// Mined first, then by how often they were read: the mined ones are the
/// reader's own claims and the read ones are the dictionary's misses.
pub async fn promotion_candidates(
    k: &Knowledge,
    min_encounters: i64,
    limit: i64,
) -> Result<Vec<PromotionCandidate>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT headword, reading, mined, encounter_count, in_reference \
         FROM vocabulary \
         WHERE promoted = 0 AND in_master = 0 AND in_name = 0 \
           AND (mined = 1 OR encounter_count >= ?) \
         ORDER BY mined DESC, encounter_count DESC LIMIT ?",
    )
    .bind(min_encounters)
    .bind(limit)
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| PromotionCandidate {
            term: Term {
                headword: r.get("headword"),
                reading: r.get("reading"),
            },
            mined: r.get::<i64, _>("mined") != 0,
            encounter_count: r.get("encounter_count"),
            in_reference: r.get::<i64, _>("in_reference") != 0,
        })
        .collect())
}

/// How many candidates there are in total, since the queue is only its head.
pub async fn promotion_pending(k: &Knowledge, min_encounters: i64) -> Result<i64, sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vocabulary \
         WHERE promoted = 0 AND in_master = 0 AND in_name = 0 \
           AND (mined = 1 OR encounter_count >= ?)",
    )
    .bind(min_encounters)
    .fetch_one(k.pool())
    .await?;
    Ok(n)
}

/// Count these terms as vocabulary though no master dictionary lists them.
///
/// An assertion, like `status`: only ever written from a request the reader
/// made. Never sets `status` as a side effect — promoting 冪等性 says it is a
/// word, not that it is known, and the two questions have different answers
/// for anything still in the queue.
pub async fn set_promoted(
    k: &Knowledge,
    terms: &[Term],
    promoted: bool,
) -> Result<u64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    let mut n = 0;
    for term in terms {
        n += sqlx::query("UPDATE vocabulary SET promoted = ? WHERE headword = ? AND reading = ?")
            .bind(i64::from(promoted))
            .bind(&term.headword)
            .bind(&term.reading)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(n)
}

/// What [`repair_empty_readings`] did.
#[derive(Debug, Clone, Default)]
pub struct ReadingRepair {
    /// Rows re-keyed onto their real reading, no twin in the way.
    pub rekeyed: u64,
    /// Rows merged into a twin the tokenizer had already made.
    pub merged: u64,
    /// Left alone: no dictionary gives the headword a reading, or several do
    /// and guessing would assert one.
    pub unresolved: u64,
}

/// Re-key the empty-reading rows that should never have had one.
///
/// Pass 1 stores an empty reading when the master dictionary offers no
/// candidate. For a kana headword that is correct and is the ledger's own
/// convention. For a *kanji* headword it is a defect with two shapes, both
/// found in live data:
///
/// - **a duplicate** — 復号 exists twice, once as `復号/ふくごう` carrying the
///   encounters the tokenizer recorded and once as `復号/` carrying the
///   `known` the Anki import wrote. One word, two rows, and every count split
///   between them.
/// - **an orphan** — 冪等性 exists only as `冪等性/`, on a key nothing else
///   will ever write to, so its judgement is stranded.
///
/// Both are fixed by asking a *reference* dictionary for the reading the
/// master could not supply, then merging onto the properly-keyed row or
/// re-keying in place. A headword no dictionary reads, or one several read
/// differently, is left exactly as it is: guessing here would fabricate the
/// key everything else joins on.
///
/// Merging keeps the stronger assertion — a judged status is never replaced by
/// `new` — and sums the counts, since both rows counted the same word.
/// Idempotent: a second run finds nothing left to move.
pub async fn repair_empty_readings(k: &Knowledge) -> Result<ReadingRepair, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT headword FROM vocabulary WHERE reading = ''")
        .fetch_all(k.pool())
        .await?;

    let mut out = ReadingRepair::default();
    for (headword,) in rows {
        // A kana headword storing no reading is the convention, not a bug.
        if kana::is_all_kana(&headword) {
            continue;
        }
        let readings = super::dictionaries::any_readings(k.pool(), &headword).await?;
        let [reading] = readings.as_slice() else {
            out.unresolved += 1;
            continue;
        };
        let target = Term::new(headword.clone(), reading);
        let source = Term {
            headword: headword.clone(),
            reading: String::new(),
        };
        if target == source {
            continue;
        }

        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM vocabulary WHERE headword = ? AND reading = ?")
                .bind(&target.headword)
                .bind(&target.reading)
                .fetch_optional(k.pool())
                .await?;

        let mut tx = k.pool().begin().await?;
        if exists.is_some() {
            // Fold the stranded row into the real one. `status` moves only
            // when the target has nothing asserted, so a seed can never
            // overrule a judgement here either.
            sqlx::query(
                "UPDATE vocabulary AS t SET \
                     status = CASE WHEN t.status = 'new' THEN s.status ELSE t.status END, \
                     status_ts = CASE WHEN t.status = 'new' THEN s.status_ts ELSE t.status_ts END, \
                     status_source = CASE WHEN t.status = 'new' THEN 'merge' \
                                          ELSE t.status_source END, \
                     mined = MAX(t.mined, s.mined), \
                     promoted = MAX(t.promoted, s.promoted), \
                     encounter_count = t.encounter_count + s.encounter_count, \
                     lookup_count = t.lookup_count + s.lookup_count \
                 FROM (SELECT * FROM vocabulary WHERE headword = ? AND reading = '') AS s \
                 WHERE t.headword = ? AND t.reading = ?",
            )
            .bind(&headword)
            .bind(&target.headword)
            .bind(&target.reading)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM vocabulary WHERE headword = ? AND reading = ''")
                .bind(&headword)
                .execute(&mut *tx)
                .await?;
            out.merged += 1;
        } else {
            sqlx::query("UPDATE vocabulary SET reading = ? WHERE headword = ? AND reading = ''")
                .bind(&target.reading)
                .bind(&headword)
                .execute(&mut *tx)
                .await?;
            out.rekeyed += 1;
        }
        tx.commit().await?;
    }
    Ok(out)
}

/// The two derived states an unjudged row can be in.
///
/// `status` records what the *reader* asserted; encounters record what the
/// *reading* did. They are orthogonal, so a row is legitimately "never judged"
/// and "met 53 times" at once — which is what おじ is. Reporting the whole
/// unjudged bucket as `new` therefore mislabels almost all of it: on
/// 2026-07-29, 2,143 of 2,155 unjudged rows had been met.
///
/// Both figures are derived and neither is stored. Storing them would add a
/// writer to a column only the reader may write, and freeze a threshold that
/// should stay re-tunable over the whole history like every other read-stats
/// derivation.
#[derive(Debug, Clone, Default)]
pub struct UnjudgedCounts {
    /// Met while reading, never judged. The honest name for most of `new`.
    pub seen: i64,
    /// Never met and never judged — a row some import created for a word the
    /// reading has not reached yet. Genuinely new.
    pub never_met: i64,
    /// Met at least `min_encounters` times and counting as vocabulary: what
    /// the sweep can actually offer. A subset of `seen`.
    ///
    /// The lookup half of the triage rule is deliberately not applied — this
    /// counts what is ready to be *asked about*, not what would be ticked.
    pub ready: i64,
    /// How many of `never_met` count as vocabulary, so a display can split the
    /// stored `new` bucket's vocabulary column the same way it splits its total.
    pub never_met_vocab: i64,
}

pub async fn unjudged_counts(
    k: &Knowledge,
    min_encounters: i64,
) -> Result<UnjudgedCounts, sqlx::Error> {
    let row = sqlx::query(&format!(
        "SELECT \
           COALESCE(SUM(CASE WHEN encounter_count > 0 THEN 1 ELSE 0 END), 0) AS seen, \
           COALESCE(SUM(CASE WHEN encounter_count = 0 THEN 1 ELSE 0 END), 0) AS never_met, \
           COALESCE(SUM(CASE WHEN encounter_count >= ? AND {COUNTS_AS_VOCAB} \
                             THEN 1 ELSE 0 END), 0) AS ready, \
           COALESCE(SUM(CASE WHEN encounter_count = 0 AND {COUNTS_AS_VOCAB} \
                             THEN 1 ELSE 0 END), 0) AS never_met_vocab \
         FROM vocabulary WHERE status = 'new'"
    ))
    .bind(min_encounters)
    .fetch_one(k.pool())
    .await?;
    Ok(UnjudgedCounts {
        seen: row.get("seen"),
        never_met: row.get("never_met"),
        ready: row.get("ready"),
        never_met_vocab: row.get("never_met_vocab"),
    })
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

    /// Pins the plan for the same reason as the test above: `frequency_pending`
    /// hung past a 15s timeout against the real database before the reading
    /// count subquery was pinned to `idx_dictionary_entries_lookup` —
    /// `COUNT(DISTINCT reading)` made the planner prefer
    /// `idx_dictionary_entries_reading`, which can only seek on
    /// `dictionary_id` and turned the `term` filter into a per-row scan of
    /// every master entry. A correctness test alone would not have caught it:
    /// the counts were right, just minutes late.
    #[tokio::test]
    async fn frequency_pending_seeks_the_dictionary_index_for_readings_too() {
        let k = temp().await;
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(&format!(
            "EXPLAIN QUERY PLAN SELECT \
                 COALESCE(SUM(CASE WHEN readings <= 1 THEN 1 ELSE 0 END), 0), \
                 COALESCE(SUM(CASE WHEN readings > 1 THEN 1 ELSE 0 END), 0) \
             FROM ( \
                 SELECT df.term, \
                        (SELECT COUNT(DISTINCT de.reading) FROM dictionary_entries de \
                         INDEXED BY idx_dictionary_entries_lookup \
                         WHERE de.dictionary_id = 1 AND de.term = df.term) AS readings \
                 FROM dictionary_frequency df \
                 WHERE {FREQUENCY_FILTER} \
                 GROUP BY df.term HAVING MIN(df.frequency) <= 6000)"
        ))
        .bind(2)
        .bind(1)
        .fetch_all(k.pool())
        .await
        .unwrap();
        let detail: Vec<&str> = plan.iter().map(|r| r.3.as_str()).collect();
        assert!(
            detail.iter().any(|d| d
                .contains("SEARCH de USING INDEX idx_dictionary_entries_lookup")
                && d.contains("dictionary_id=?")
                && d.contains("term=?")),
            "the reading-count subquery must seek on (dictionary_id, term), not just dictionary_id: {detail:?}"
        );
    }

    #[tokio::test]
    async fn frequency_queue_excludes_homographs_and_resolves_the_rest() {
        let k = temp().await;
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master'), (2, 'BCCWJ', '/b.zip', 'reference');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '二十', 'にじゅう', '[]'), (1, '二十', 'はたち', '[]'), \
                        (1, '犬', 'いぬ', '[]');\
             INSERT INTO dictionary_frequency (dictionary_id, term, frequency) \
                 VALUES (2, '二十', 10), (2, '犬', 20);",
        )
        .execute(k.pool())
        .await
        .unwrap();

        let pending = frequency_pending(&k, 2, 1, 100).await.unwrap();
        assert_eq!(pending.committable, 1, "only 犬 has one reading");
        assert_eq!(pending.ambiguous, 1, "二十 is にじゅう or はたち");

        let queue = frequency_queue(&k, 2, 1, 100, 10, 0).await.unwrap();
        assert_eq!(queue.len(), 1, "二十 must not occupy a page slot");
        assert_eq!(queue[0].term, "犬");
        assert_eq!(queue[0].reading, "いぬ");

        // A term the reader has already judged (however that happened) drops
        // out on the next call, exactly like the triage queue's own rule.
        set_status(&k, &Term::new("犬", "いぬ"), Status::Unknown, 1.0)
            .await
            .unwrap();
        assert_eq!(
            frequency_queue(&k, 2, 1, 100, 10, 0).await.unwrap().len(),
            0
        );
        assert_eq!(
            frequency_pending(&k, 2, 1, 100).await.unwrap().committable,
            0
        );
    }

    #[tokio::test]
    async fn a_word_already_judged_under_another_reading_is_not_offered_again() {
        let k = temp().await;
        record_encounters(
            &k,
            &[
                enc("皆", "みな", 233, 0.0),
                enc("皆", "みんな", 165, 0.0),
                enc("空", "そら", 90, 0.0),
                enc("鍵", "かぎ", 40, 0.0),
            ],
        )
        .await
        .unwrap();
        sqlx::query("UPDATE vocabulary SET in_master = 1")
            .execute(k.pool())
            .await
            .unwrap();
        set_status(&k, &Term::new("皆", "みな"), Status::Known, 1.0)
            .await
            .unwrap();

        let offered: Vec<String> = triage_queue(&k, 1, None, 50)
            .await
            .unwrap()
            .iter()
            .map(|r| format!("{}/{}", r.term.headword, r.term.reading))
            .collect();
        assert!(
            !offered.iter().any(|t| t == "皆/みんな"),
            "the same word again, in a second reading: {offered:?}"
        );
        assert!(offered.iter().any(|t| t == "空/そら"));
        assert!(offered.iter().any(|t| t == "鍵/かぎ"));
        // The count has to agree with the queue, or it promises rows nobody
        // will be shown.
        assert_eq!(
            triage_pending(&k, 1, None).await.unwrap().0,
            offered.len() as i64
        );
    }

    #[tokio::test]
    async fn a_judgement_follows_its_word_to_the_key_the_ingest_now_uses() {
        let k = temp().await;
        let (old, new) = (Term::new("いっぱい", ""), Term::new("一杯", "いっぱい"));
        set_status(&k, &old, Status::Known, 5.0).await.unwrap();
        record_encounters(&k, &[enc("一杯", "いっぱい", 40, 0.0)])
            .await
            .unwrap();

        assert!(carry_judgement(&k, &old, &new).await.unwrap());
        let row = fetch(&k, &new).await.unwrap().unwrap();
        assert_eq!(row.status, Status::Known);
        assert_eq!(row.status_ts, Some(5.0));
        assert_eq!(row.encounter_count, 40, "counts came from the rebuild");
        assert!(fetch(&k, &old).await.unwrap().is_none(), "the old key goes");
    }

    #[tokio::test]
    async fn a_judgement_with_nowhere_to_go_is_kept_not_dropped() {
        let k = temp().await;
        let (old, gone) = (Term::new("いっぱい", ""), Term::new("一杯", "いっぱい"));
        set_status(&k, &old, Status::Known, 5.0).await.unwrap();
        // No row for the new key: the word is not in the reading any more.
        assert!(!carry_judgement(&k, &old, &gone).await.unwrap());
        assert_eq!(
            fetch(&k, &old).await.unwrap().unwrap().status,
            Status::Known,
            "deleting it would lose an answer to save a row"
        );
    }

    #[tokio::test]
    async fn carrying_never_overrules_what_the_reader_said_about_the_target() {
        let k = temp().await;
        let (old, new) = (Term::new("あげる", ""), Term::new("上げる", "あげる"));
        set_status(&k, &old, Status::Known, 5.0).await.unwrap();
        set_status(&k, &new, Status::Unknown, 9.0).await.unwrap();

        assert!(!carry_judgement(&k, &old, &new).await.unwrap());
        assert_eq!(
            fetch(&k, &new).await.unwrap().unwrap().status,
            Status::Unknown,
            "the target's own assertion stands"
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

        let queue = triage_queue(&k, 3, None, 100).await.unwrap();
        let words: Vec<&str> = queue.iter().map(|r| r.term.headword.as_str()).collect();
        assert_eq!(
            words,
            vec!["憂鬱"],
            "齟齬 is under the floor, っっ is not a word, 読む is already judged"
        );

        // The floor is the only thing keeping 齟齬 out, so lowering it lets it in.
        assert_eq!(triage_queue(&k, 1, None, 100).await.unwrap().len(), 2);
        // …and っっ stays out at any floor, despite being the most-met row.
        assert!(
            !triage_queue(&k, 1, None, 100)
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

        let (total, preselected) = triage_pending(&k, 3, None).await.unwrap();
        assert_eq!(total, 2, "both are unjudged vocabulary above the floor");
        assert_eq!(preselected, 1, "only 憂鬱 was never looked up");
    }

    /// The periodic sweep's scoping: a word not read since the last sweep is
    /// not in this batch, however many times it was met before it.
    #[tokio::test]
    async fn a_scoped_sweep_offers_only_what_has_been_read_since_the_mark() {
        let k = temp().await;
        record_encounters(
            &k,
            &[
                enc("憂鬱", "ユウウツ", 9, 100.0),
                enc("齟齬", "ソゴ", 9, 20.0),
            ],
        )
        .await
        .unwrap();
        sqlx::query("UPDATE vocabulary SET in_master = 1")
            .execute(k.pool())
            .await
            .unwrap();

        let scoped = triage_queue(&k, 3, Some(50.0), 100).await.unwrap();
        let words: Vec<&str> = scoped.iter().map(|r| r.term.headword.as_str()).collect();
        assert_eq!(words, vec!["憂鬱"], "齟齬 was last read before the mark");
        assert_eq!(triage_pending(&k, 3, Some(50.0)).await.unwrap(), (1, 1));

        // Unscoped, both are still there — the mark narrows the batch, it does
        // not judge or retire anything.
        assert_eq!(triage_queue(&k, 3, None, 100).await.unwrap().len(), 2);

        // A mark past everything empties the batch rather than falling back to
        // the whole queue: "nothing new since you last swept" is the answer.
        let swept_past = triage_queue(&k, 3, Some(1000.0), 100).await.unwrap();
        assert!(swept_past.is_empty());
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
                "test",
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

    /// A bulk seed carries judgements made elsewhere and months ago. A row
    /// already saying `unknown` carries one made here, with the word on
    /// screen — and the first jiten import quietly turned 16 of those into
    /// `known` before this guard existed.
    #[tokio::test]
    async fn a_seed_fills_new_rows_and_leaves_judged_ones_alone() {
        let k = temp().await;
        let judged = Term::new("辛い", "からい");
        let untouched = Term::new("零れ落ちる", "こぼれおちる");
        set_status(&k, &judged, Status::Unknown, 1.0).await.unwrap();
        set_status(&k, &untouched, Status::New, 1.0).await.unwrap();

        let fresh = Term::new("こぼれ落ちる", "こぼれおちる");
        seed_status_each(
            &k,
            &[
                (judged.clone(), Status::Known),
                (untouched.clone(), Status::Known),
                (fresh.clone(), Status::Known),
            ],
            9.0,
        )
        .await
        .unwrap();

        assert_eq!(
            get(&k, &judged).await.status,
            Status::Unknown,
            "a judgement survives a seed"
        );
        assert_eq!(
            get(&k, &untouched).await.status,
            Status::Known,
            "an unjudged row is filled"
        );
        assert_eq!(
            get(&k, &fresh).await.status,
            Status::Known,
            "a missing row is created"
        );
    }

    /// Running it twice must land where running it once did.
    #[tokio::test]
    async fn seeding_is_idempotent() {
        let k = temp().await;
        let t = Term::new("零れ落ちる", "こぼれおちる");
        let seed = [(t.clone(), Status::Known)];
        seed_status_each(&k, &seed, 1.0).await.unwrap();
        set_status(&k, &t, Status::Unknown, 2.0).await.unwrap();
        seed_status_each(&k, &seed, 3.0).await.unwrap();
        assert_eq!(
            get(&k, &t).await.status,
            Status::Unknown,
            "a re-run cannot undo a judgement made between the two"
        );
    }

    async fn get(k: &Knowledge, term: &Term) -> VocabRow {
        fetch(k, term).await.unwrap().expect("row exists")
    }
}
