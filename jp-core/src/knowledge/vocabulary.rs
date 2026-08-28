//! The vocabulary ledger — what I know, one row per term.
//!
//! Every other table in `knowledge.db` records an event; this one records a
//! state. Counts live on the row rather than being derived per query because
//! the highlighter looks one up per token, per line, as it streams.
//!
//! | fact | writer | shape |
//! |---|---|---|
//! | encounters | [`record_encounters`], from kotodex-server's watermarked ingest | incremental |
//! | mined | [`sync_mined`], from the `anki_notes` snapshot | wholesale |
//! | lookups | [`sync_lookup_counts`], from `lookups` | wholesale |
//! | dictionary flags | [`refresh_dictionary_flags`] | wholesale |
//! | **status** | the reader, via [`set_status`] | never by a sync |
//!
//! The wholesale three mirror a table that already owns the truth. Encounters
//! are incremental because re-tokenizing all of `lines` on every Anki refresh
//! would be minutes of CPU. `status` is assertions only.

use std::collections::HashMap;

use sqlx::{Row, SqlitePool};

use super::Knowledge;
use crate::text::kana;

/// What counts toward "I know N words", as a SQL predicate.
///
/// One definition, used by every figure that reports a vocabulary size, so they
/// cannot drift apart. The master dictionary's answer, **or** the reader's
/// override where the master has nothing to say — Sankoku carries no 冪等性, and
/// admitting JMdict instead would drag in every idiom and variant spelling.
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

    /// `learning` and `name` were removed: `learning` duplicated `mined`, and
    /// names never reach the ledger. Either still in the database reads back as
    /// `new` via [`Status::parse`].
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
    /// Normalize a `(headword, reading)` pair, so the same word lands on one
    /// row whether it came from a VN line, a pasted article or an Anki card:
    ///
    /// 1. The reading folds to hiragana — Sudachi emits katakana, the
    ///    dictionaries hold hiragana, and the ledger joins them.
    /// 2. A kana-only headword stores an empty reading, since there the two
    ///    strings are the same fact.
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
    /// Listed by some dictionary that is neither the master nor a name list —
    /// `reference` and `standard` both, which is every remaining role.
    ///
    /// One flag rather than two because nothing asks the two apart: every
    /// consumer of these is [`is_word`](VocabRow::is_word), and the vocabulary
    /// scale is [`in_master`](VocabRow::in_master) alone either way.
    pub in_reference: bool,
    /// Frequency rank, only where the query asked for it ([`QueueOrder::Frequency`]).
    /// `None` also means "the frequency list does not carry this word", so it
    /// cannot be read as a rank on its own.
    pub freq_rank: Option<i64>,
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
        freq_rank: r.try_get("freq_rank").ok().flatten(),
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
/// meeting a word again says nothing about whether it is known. Promotion is a
/// decision the reader makes, never something ingest does behind them.
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
/// Matched on headword alone, because that is all Anki has — so a homograph
/// marks every reading of itself as mined. That is the limit of the source, and
/// it fails safe for the highlighter; the fix would be a reading on the card,
/// not a guess here.
///
/// Returns how many rows now carry the flag.
pub async fn sync_mined(k: &Knowledge) -> Result<i64, sqlx::Error> {
    let mut tx = k.pool().begin().await?;
    sqlx::query("UPDATE vocabulary SET mined = 0 WHERE mined = 1")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        // `headword` is the card's spelling normalized the way the ledger keys
        // it; matching the raw `vocab` lost every card whose spelling
        // normalizes — 検死 never marked 検屍. Empty means a snapshot older than
        // the column, which falls back to the old behaviour until the next
        // refresh fills it.
        "UPDATE vocabulary SET mined = 1 \
         WHERE headword IN (SELECT COALESCE(NULLIF(headword, ''), vocab) FROM anki_notes)",
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
/// Also matched on headword alone, since Yomitan sends no reading. Recomputed
/// rather than incremented, so discarding lines or fixing the capture guard is
/// reflected on the next refresh instead of leaving a count that only grows.
pub async fn sync_lookup_counts(k: &Knowledge) -> Result<(), sqlx::Error> {
    sqlx::query(
        // Against the normalized key, not the spelling Yomitan sent: a lookup
        // of 検死 belongs to 検屍, the row the reader actually meets. Empty
        // means a row the backfill has not reached, which falls back to the
        // spelling and so behaves as it did before.
        "UPDATE vocabulary SET lookup_count = \
             (SELECT COUNT(*) FROM lookups \
               WHERE COALESCE(NULLIF(lookups.headword, ''), lookups.term) \
                     = vocabulary.headword)",
    )
    .execute(k.pool())
    .await?;
    Ok(())
}

/// Recompute the three dictionary flags from `dictionary_entries` + the roles.
///
/// A term matches a dictionary if the dictionary lists its headword. The
/// reading deliberately need not match: a dictionary spelling one differently
/// would make a real word look like tokenizer noise, and these flags answer "is
/// this a word", not "is this exact pair attested".
///
/// **Each subquery must be able to seek `idx_dictionary_entries_lookup`.** That
/// index is `(dictionary_id, term)`, so filtering on `d.role` through a join
/// leaves its leading column unconstrained and SQLite scans every entry per
/// ledger row. Resolving the role to its ids first makes each subquery a seek.
/// The join form took six minutes and held the write lock throughout, failing
/// every concurrent write with "database is locked"; this runs in 15 ms.
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
    let clause = |roles: &[&str]| {
        let list = roles
            .iter()
            .map(|r| format!("'{r}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let of_role = format!("(SELECT id FROM dictionaries WHERE role IN ({list}))");
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
        clause(&["master"]),
        clause(&["name"]),
        // **`standard` counts here.** The flag answers "some dictionary has
        // this", and 明鏡 and 小学館 are dictionaries — they are already trusted
        // with the harder question of where a word ends. Folded into this one
        // rather than given a column of their own because nothing downstream
        // tells a reference listing from a standard one: every consumer asks
        // `is_word`, and the vocabulary scale is `in_master` alone either way.
        clause(&["reference", "standard"]),
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
/// For the highlighter, which asks about a line's dozen words between the line
/// arriving and being drawn. One query rather than a [`fetch`] per token: the
/// round trips are what would show beside a 30ms poll.
///
/// Terms with no row are absent from the map — that is the ordinary state of a
/// line hooked a second ago, not an error.
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
/// `UNJUDGED_HEADWORD`'s rule and `work_terms::IS_KNOWN`'s, for a caller holding
/// terms rather than writing SQL. The ledger keys on `(headword, reading)` for
/// homographs, but most pairs are one word the dictionary lists twice, and an
/// inflected form is a third way to a second row (通れ → 通る/とおれる, beside
/// 通る/とおる).
///
/// The reading comes back too, because a caller that lets the reader *act* on
/// the word must write to the row carrying the assertion — taking 通る back has
/// to hit 通る/とおる, not whatever the tokenizer produced. Readings are ordered
/// so the answer is stable when a headword is known under two.
///
/// Not [`fetch_many`] with a looser key: this asks whether the *word* is known
/// at all, and a caller wanting one row's status still gets it from there.
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
/// Three filters:
///
/// - **`status = 'new'`** — only the never-judged. Re-asking about a word the
///   reader has ruled on is how a triage pass loses their trust.
/// - **[`COUNTS_AS_VOCAB`]** — master-dictionary terms plus the reader's
///   promotions. The rest are phrase headwords, names and tokenizer noise:
///   they belong in the ledger, but not in a queue judged one row at a time.
/// - **`encounter_count >= min_encounters`** — a word met twice is not yet
///   evidence of anything.
///
/// Ordered by [`QueueOrder`] — encounter count by default, since the words met
/// most are the ones every downstream feature hits most.
///
/// **A word judged under one of its readings is not offered under another.**
/// The ledger keys on `(headword, reading)` because 空 is そら or から, but most
/// pairs are one word the dictionary lists twice (言う as いう and ゆう). This
/// only stops the asking — the second row stays `new`, which is true, since
/// inheriting the status would be a sync writing `status`.
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
/// every load; that trade is only worth making if the batches come out noisy.
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

/// What the sweep's page is sorted by. Two different questions over the same
/// rows: which words *this* reading keeps producing, and which words are common
/// in Japanese at all — the second reaches a word met three times that everyone
/// else meets constantly, which the encounter order buries.
#[derive(Debug, Clone, Copy)]
pub enum QueueOrder {
    Encounters,
    /// Frequency rank, commonest first. A row the frequency list does not carry
    /// sorts last rather than dropping out: the ordering is a view of the
    /// queue, and must not hide rows the count promises.
    Frequency {
        freq_id: i64,
    },
}

pub async fn triage_queue(
    k: &Knowledge,
    min_encounters: i64,
    since_ts: Option<f64>,
    order: QueueOrder,
    limit: i64,
) -> Result<Vec<VocabRow>, sqlx::Error> {
    let (select, order_by) = match order {
        QueueOrder::Encounters => ("SELECT *".to_string(), "encounter_count DESC, headword"),
        QueueOrder::Frequency { freq_id } => {
            let lex = super::dictionaries::lexeme_dictionary(k.pool()).await?;
            let rank = word_rank_sql(lex, Some(freq_id), "vocabulary");
            (
                format!("SELECT *, {rank} AS freq_rank"),
                "freq_rank IS NULL, freq_rank, encounter_count DESC, headword",
            )
        }
    };
    let sql = format!(
        "{select} FROM vocabulary WHERE {} ORDER BY {order_by} LIMIT ?",
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
/// Separate from [`triage_queue`] because a `limit`ed queue cannot answer "how
/// many are left". `since_ts` must match the queue's, or the header counts a
/// different batch from the one on screen.
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
/// from "met 47 times and looked up twelve of them", and the second is a word
/// the reader does *not* have. So **a single lookup disqualifies the default**,
/// whatever the encounter count.
///
/// It decides a default, never a write: the reader submits the judgement.
pub fn preselects_known(row: &VocabRow, min_encounters: i64) -> bool {
    row.encounter_count >= min_encounters && row.lookup_count == 0
}

/// How common the *word* is, as an SQL expression over a `vocabulary` row.
///
/// Not `MIN(frequency) WHERE term = headword`. BCCWJ is annotated with UniDic,
/// which normalises the spelling, so the corpus files お辞儀 under 御辞儀 and
/// asking under Sankoku's spelling answers 405,782 for a word the corpus ranks
/// 12,272. The rank is taken over every spelling the entry carries **for this
/// reading**, which is what the corpus was counting all along.
///
/// Grouping by reading and not by entry is what keeps 明日【みょうにち】at
/// 209,173 rather than inheriting 明日【あした】's rank — a rare reading is a
/// different word to know, where a rare spelling is not.
///
/// The form's own rank stays in the `MIN`: a row the reference dictionary
/// cannot resolve has no siblings and must not lose the rank it does have.
///
/// The ids are formatted in rather than bound — they come from `dictionaries`,
/// and the callers number their placeholders differently.
fn word_rank_sql(lex: Option<i64>, corpus: Option<i64>, alias: &str) -> String {
    let Some(corpus) = corpus else {
        return "NULL".into();
    };
    let hw = format!("{alias}.headword");
    // A kana headword stores no reading — `Term::new` refuses to write いる
    // twice — so the reading has to be spelled out before it can be matched
    // against a dictionary that always carries one.
    let rd = format!("CASE WHEN {alias}.reading = '' THEN {hw} ELSE {alias}.reading END");
    // Two subqueries and a MIN, rather than one over a `term IN (...)` set: the
    // set form makes `dictionary_frequency` unreachable by index and costs a
    // second per row — 4.7s for five rows against 0.18s for the whole ledger.
    let form = format!(
        "(SELECT MIN(f.frequency) FROM dictionary_frequency f \
           WHERE f.dictionary_id = {corpus} AND f.term = {hw} \
             AND (f.reading = {rd} OR f.reading = ''))"
    );
    // No reference dictionary loaded is no way to know what a word's other
    // spellings are, so the form's own rank is all there is.
    let Some(lex) = lex else { return form };
    let said = |j: &str| format!("COALESCE(NULLIF({j}.reading, ''), {j}.term)");
    let entry = format!(
        "(SELECT j0.sequence FROM dictionary_entries j0 \
           WHERE j0.dictionary_id = {lex} AND j0.term = {hw} AND {} = {rd} \
           ORDER BY j0.score DESC LIMIT 1)",
        said("j0")
    );
    let siblings = format!(
        "(SELECT MIN(f.frequency) FROM dictionary_entries j \
            JOIN dictionary_frequency f \
              ON f.dictionary_id = {corpus} AND f.term = j.term \
             AND (f.reading = {rd} OR f.reading = '') \
           WHERE j.dictionary_id = {lex} AND j.sequence = {entry} AND {} = {rd})",
        said("j")
    );
    // UniDic normalises the *reading* too, not only the spelling: it reads
    // 不器用 as ふきよう (14,328) where the dictionaries say ぶきよう (405,782),
    // and 日本 as ニッポン. So when the entry has one reading, whatever reading
    // the corpus recorded for its spellings must be that reading — there is no
    // other word it could have been counting. An entry with two is left alone,
    // which is what keeps 明日【みょうにち】off 明日【あす】's rank.
    let one_reading = format!(
        "(SELECT MIN(f.frequency) FROM dictionary_entries j \
            JOIN dictionary_frequency f \
              ON f.dictionary_id = {corpus} AND f.term = j.term \
           WHERE j.dictionary_id = {lex} AND j.sequence = {entry} \
             AND (SELECT COUNT(DISTINCT {}) FROM dictionary_entries j2 \
                   WHERE j2.dictionary_id = {lex} AND j2.sequence = j.sequence) = 1)",
        said("j2")
    );
    // An aggregate `MIN` over the three, rather than the scalar one: the scalar
    // form is NULL if any argument is, and two of these three usually are.
    format!(
        "(SELECT MIN(r) FROM (SELECT {form} AS r \
                        UNION ALL SELECT {siblings} UNION ALL SELECT {one_reading}))"
    )
}

/// A page of the ledger, filtered and ordered for reading rather than judging.
///
/// The triage passes each pick their own rows and show them one screen at a
/// time; this is the other question — "what is actually in here, and where did
/// it come from". Ordered by corpus rank because that is the axis a bulk import
/// goes wrong along: the rare end is where a threshold rule claims words that
/// were never really met.
///
/// `source` filters on `status_source`, so `seed` is a bulk import and
/// `triage` is what was judged by hand. Unranked rows sort last: BCCWJ is a
/// written corpus and its silence about この野郎 is a gap in the corpus, not a
/// statement about the word.
/// `collapse` shows one row per *word* rather than per ledger row — the
/// question this view is for. The ledger keys on forms because the tokenizer
/// emits forms, but 元々 and 元元 are one word and listing both is listing the
/// database rather than the vocabulary. The survivor is the entry's
/// best-scored spelling, which is the one the word is normally written with,
/// and `forms` says how many it stands for.
pub async fn browse(
    k: &Knowledge,
    status: Option<&str>,
    source: Option<&str>,
    rarest_first: bool,
    collapse: bool,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<BrowseRow>), sqlx::Error> {
    // The corpus id is resolved here rather than reached through
    // `JOIN dictionaries ON title = 'BCCWJ'` inside the correlated subquery,
    // which costs `idx_dictionary_frequency_lookup` — the plan drops to a bare
    // SEARCH per row, and the page never returns.
    let corpus = super::dictionaries::by_title(k.pool(), "BCCWJ")
        .await?
        .map(|d| d.id);

    let lex = super::dictionaries::lexeme_dictionary(k.pool()).await?;

    // Unranked last either way. BCCWJ is written text, so its silence about
    // この野郎 is a hole in the corpus and not a claim that the word is rare —
    // sorting those to the top of "rarest first" would fill the page with the
    // one thing the rank cannot speak to.
    let order = if rarest_first {
        "rank IS NULL, rank DESC"
    } else {
        "rank IS NULL, rank ASC"
    };
    // A form with no entry id groups only with itself, so an unresolvable row is
    // never merged into another word — it stands alone, exactly as the lexeme
    // layer's third rule has it.
    //
    // The entry id is taken off the form's best-scored row rather than as a
    // `MIN(sequence)`: SQLite serves a MIN over that column from
    // `idx_dictionary_entries_sequence`, which is keyed on `dictionary_id`
    // alone, so the term never reaches an index and every ledger row costs a
    // partial scan of 400k entries.
    let group = "COALESCE(CAST(seq AS TEXT), headword || CHAR(31) || reading)";
    // Collapsed, the filter picks *words*, not rows. 元々 was judged by hand and
    // 元元 came from the import; filtering the rows first left 元元 standing
    // alone as its own word, which is the database showing through again. So a
    // word is shown when any of its spellings matches, and the row shown is one
    // that did.
    let survivor = if collapse {
        "WHERE n = 1 AND matched = 1"
    } else {
        "WHERE hit = 1"
    };
    let rank = word_rank_sql(lex, corpus, "v");
    let sql = format!(
        "WITH picked AS ( \
             SELECT v.headword, v.reading, v.status, v.status_source, v.encounter_count, \
                    v.lookup_count, v.mined, \
                    (CASE WHEN (?1 IS NULL OR v.status = ?1) \
                            AND (?2 IS NULL OR v.status_source = ?2) \
                          THEN 1 ELSE 0 END) AS hit, \
                    (SELECT j.sequence FROM dictionary_entries j \
                      WHERE j.dictionary_id = ?3 AND j.term = v.headword \
                        AND (COALESCE(NULLIF(j.reading, ''), j.term) = v.reading \
                             OR v.reading = '') \
                      ORDER BY j.score DESC LIMIT 1) AS seq, \
                    (SELECT MAX(j.score) FROM dictionary_entries j \
                      WHERE j.dictionary_id = ?3 AND j.term = v.headword \
                        AND (COALESCE(NULLIF(j.reading, ''), j.term) = v.reading \
                             OR v.reading = '')) AS score, \
                    {rank} AS rank \
               FROM vocabulary v), \
          grouped AS ( \
             SELECT *, \
                    ROW_NUMBER() OVER (PARTITION BY {group} \
                        ORDER BY hit DESC, score DESC, encounter_count DESC, headword) AS n, \
                    COUNT(*) OVER (PARTITION BY {group}) AS forms, \
                    MAX(hit) OVER (PARTITION BY {group}) AS matched \
               FROM picked) \
         SELECT *, COUNT(*) OVER () AS total FROM grouped {survivor} \
          ORDER BY {order}, headword LIMIT ?4 OFFSET ?5"
    );
    let rows = sqlx::query(&sql)
        .bind(status)
        .bind(source)
        .bind(lex)
        .bind(limit)
        .bind(offset)
        .fetch_all(k.pool())
        .await?;

    let total = rows.first().map(|r| r.get::<i64, _>("total")).unwrap_or(0);
    Ok((
        total,
        rows.iter()
            .map(|r| BrowseRow {
                term: Term::new(
                    r.get::<String, _>("headword"),
                    &r.get::<String, _>("reading"),
                ),
                status: r.get("status"),
                source: r.get("status_source"),
                encounter_count: r.get("encounter_count"),
                lookup_count: r.get("lookup_count"),
                mined: r.get::<i64, _>("mined") != 0,
                rank: r.get("rank"),
                forms: r.get("forms"),
            })
            .collect(),
    ))
}

/// See [`browse`].
#[derive(Debug, Clone)]
pub struct BrowseRow {
    pub term: Term,
    pub status: String,
    pub source: Option<String>,
    pub encounter_count: i64,
    pub lookup_count: i64,
    pub mined: bool,
    pub rank: Option<i64>,
    /// How many ledger rows this one stands for — spellings of the same word.
    pub forms: i64,
}

/// When each currently-known form was **first** asserted known, from the event
/// log.
///
/// First and not last: a rebuild or a re-import re-stamps a word that was
/// already known, and dating the word by that would collapse the whole history
/// onto the day of the last bulk pass. Restricted to rows that are known *now*,
/// so a word taken back leaves no step in the curve — the answer is always what
/// the ledger says today, decomposed by when each claim was made.
///
/// A word known before the event log existed has no row and is absent; the
/// curve simply starts where the log does.
pub async fn first_known_at(k: &Knowledge) -> Result<Vec<(Term, f64)>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "SELECT v.headword, v.reading, MIN(e.ts) AS ts \
         FROM vocabulary v \
         JOIN vocabulary_events e \
           ON e.headword = v.headword AND e.reading = v.reading AND e.status = 'known' \
         WHERE v.status = 'known' AND {COUNTS_AS_VOCAB} \
         GROUP BY v.headword, v.reading"
    ))
    .fetch_all(k.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                Term {
                    headword: r.get("headword"),
                    reading: r.get("reading"),
                },
                r.get("ts"),
            )
        })
        .collect())
}

/// `source` labels the pass in the history log — `triage`, `anki`,
/// `source` labels the pass in the history log — `triage`, `anki`, `seed`. It
/// is informational; it never affects what is written to `vocabulary` itself.
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
/// Re-tokenization strands assertions: when the ledger began keying on
/// Sudachi's *normalized* form, いっぱい became 一杯 and every judgement on the
/// old spelling was left on a row nothing writes to any more.
///
/// Only the judgement moves — a stranded row's counts are zero by definition,
/// the rebuild having recomputed them onto the new key. A target carrying its
/// own assertion keeps it: this repairs a key change, it does not arbitrate
/// between two things the reader said.
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
/// After `reset_vocabulary` plus a re-ingest, a row on zero encounters is one
/// the current tokenizer no longer produces, and keeping it leaves the totals
/// counting words that are not in the reading.
///
/// Deletes only what nobody has said anything about: never judged and not in
/// Anki. An assertion and a mined card both outlive the counts.
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
/// A bulk write the reader cannot see first asks them to trust a predicate blind.
/// Same `WHERE`, every row reachable by paging — the tail is the part in
/// question, not the head.
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
/// The counterpart to the queue's `in_master` filter. The test is the negation
/// of [`VocabRow::is_word`], the lenient one, so this hits only what *nothing*
/// recognizes: tokenizer noise, not obscure vocabulary and not names.
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
/// the master scale).
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
/// The Anki import stores an empty reading when the master dictionary offers no
/// candidate. For a kana headword that is the ledger's convention; for a *kanji*
/// headword it is a defect in two shapes:
///
/// - **a duplicate** — 復号/ふくごう carries the encounters and 復号/ carries
///   the `known`, splitting one word's counts across two rows.
/// - **an orphan** — 冪等性 exists only as `冪等性/`, so its judgement sits on a
///   key nothing else will ever write to.
///
/// Both are fixed by asking a *reference* dictionary for the missing reading,
/// then merging or re-keying. A headword no dictionary reads, or one several
/// read differently, is left alone — guessing would fabricate the key
/// everything joins on.
///
/// Merging keeps the stronger assertion and sums the counts. Idempotent.
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
/// `status` records what the *reader* asserted, encounters what the *reading*
/// did. Orthogonal, so a row is legitimately "never judged" and "met 53 times"
/// at once — reporting the whole bucket as `new` mislabels nearly all of it.
///
/// Both figures are derived, never stored: storing them would add a writer to a
/// column only the reader may write, and freeze a re-tunable threshold.
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

/// Every spelling the deck mirror holds — the tokenizer's second wordhood
/// source, so a word mined yesterday is kept whole in tomorrow's lines.
///
/// The raw `vocab` and not `headword`: this feeds Sudachi's user lexicon, which
/// matches against the text, and the text spells a word the way the card does.
pub async fn mined_vocab(
    pool: &sqlx::SqlitePool,
) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT vocab FROM anki_notes")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
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

        let offered: Vec<String> = triage_queue(&k, 1, None, QueueOrder::Encounters, 50)
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

    /// A dictionary trusted to say where a word *ends* is trusted to say it is
    /// a word. 明鏡 and 小学館 are `standard` for segmentation, and leaving them
    /// out of wordhood cost 41,645 terms their span — 聞きかじり is in 明鏡 and
    /// in nothing else.
    #[tokio::test]
    async fn a_standard_dictionary_makes_a_term_a_word() {
        let k = temp().await;
        record_encounters(&k, &[enc("聞きかじり", "キキカジリ", 1, 1.0)])
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master'), (2, 'Meikyo', '/m.zip', 'standard');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (2, '聞きかじり', 'ききかじり', '[]');",
        )
        .execute(k.pool())
        .await
        .unwrap();

        refresh_dictionary_flags(&k).await.unwrap();
        let row = fetch(&k, &Term::new("聞きかじり", "キキカジリ"))
            .await
            .unwrap()
            .unwrap();
        assert!(row.is_word(), "{row:?}");
        // And it stays off the vocabulary scale, which is the master alone.
        assert!(!row.in_master, "{row:?}");
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
        let judgements: Vec<(Term, Status)> =
            terms.into_iter().map(|t| (t, Status::Known)).collect();
        assert_eq!(
            set_status_each(&k, &judgements, 9.0, "triage")
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
            freq_rank: None,
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

        let queue = triage_queue(&k, 3, None, QueueOrder::Encounters, 100)
            .await
            .unwrap();
        let words: Vec<&str> = queue.iter().map(|r| r.term.headword.as_str()).collect();
        assert_eq!(
            words,
            vec!["憂鬱"],
            "齟齬 is under the floor, っっ is not a word, 読む is already judged"
        );

        // The floor is the only thing keeping 齟齬 out, so lowering it lets it in.
        assert_eq!(
            triage_queue(&k, 1, None, QueueOrder::Encounters, 100)
                .await
                .unwrap()
                .len(),
            2
        );
        // …and っっ stays out at any floor, despite being the most-met row.
        assert!(
            !triage_queue(&k, 1, None, QueueOrder::Encounters, 100)
                .await
                .unwrap()
                .iter()
                .any(|r| r.term.headword == "っっ")
        );
    }

    /// The frequency ordering inverts the encounter one here, which is the
    /// whole point of it: a word met four times that Japanese uses constantly
    /// comes before one this reading happened to repeat. An unranked word is
    /// still offered, last.
    #[tokio::test]
    async fn the_frequency_order_offers_the_commonest_word_first() {
        let k = temp().await;
        record_encounters(
            &k,
            &[
                enc("憂鬱", "ユウウツ", 40, 1.0),
                enc("時間", "ジカン", 4, 1.0),
                enc("齟齬", "ソゴ", 9, 1.0),
            ],
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO dictionaries (id, title, source_path, role) \
                 VALUES (1, 'Sankoku', '/s.zip', 'master'), (2, 'BCCWJ', '/b.zip', 'reference');\
             INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
                 VALUES (1, '憂鬱', 'ゆううつ', '[]'), (1, '時間', 'じかん', '[]'), \
                        (1, '齟齬', 'そご', '[]');\
             INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) \
                 VALUES (2, '時間', 'じかん', 120), (2, '憂鬱', 'ゆううつ', 9000);",
        )
        .execute(k.pool())
        .await
        .unwrap();
        refresh_dictionary_flags(&k).await.unwrap();

        let bccwj = QueueOrder::Frequency { freq_id: 2 };
        let words: Vec<String> = triage_queue(&k, 1, None, bccwj, 100)
            .await
            .unwrap()
            .iter()
            .map(|r| r.term.headword.clone())
            .collect();
        assert_eq!(words, vec!["時間", "憂鬱", "齟齬"]);

        let ranks: Vec<Option<i64>> = triage_queue(&k, 1, None, bccwj, 100)
            .await
            .unwrap()
            .iter()
            .map(|r| r.freq_rank)
            .collect();
        assert_eq!(ranks, vec![Some(120), Some(9000), None]);
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

        let scoped = triage_queue(&k, 3, Some(50.0), QueueOrder::Encounters, 100)
            .await
            .unwrap();
        let words: Vec<&str> = scoped.iter().map(|r| r.term.headword.as_str()).collect();
        assert_eq!(words, vec!["憂鬱"], "齟齬 was last read before the mark");
        assert_eq!(triage_pending(&k, 3, Some(50.0)).await.unwrap(), (1, 1));

        // Unscoped, both are still there — the mark narrows the batch, it does
        // not judge or retire anything.
        assert_eq!(
            triage_queue(&k, 3, None, QueueOrder::Encounters, 100)
                .await
                .unwrap()
                .len(),
            2
        );

        // A mark past everything empties the batch rather than falling back to
        // the whole queue: "nothing new since you last swept" is the answer.
        let swept_past = triage_queue(&k, 3, Some(1000.0), QueueOrder::Encounters, 100)
            .await
            .unwrap();
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

    /// A reference dictionary that spells おじぎ two ways under one entry, and a
    /// corpus that only counted the spelling the ledger does not use — the
    /// UniDic normalisation, in miniature. 明日 rides along as the control:
    /// one entry, two readings, one of them rare.
    async fn corpus_that_spells_it_differently(k: &Knowledge) {
        let sql = "\
            INSERT INTO dictionaries (id, title, source_path, role) VALUES \
                (1, 'master', 'm.zip', 'master'), (2, 'lex', 'l.zip', 'reference'), \
                (3, 'BCCWJ', 'b.zip', 'reference'); \
            INSERT INTO dictionary_entries (dictionary_id, term, reading, score, \
                                            definitions_json, sequence) VALUES \
                (2, 'お辞儀', 'おじぎ', 200, '[]', 100), \
                (2, '御辞儀', 'おじぎ', 100, '[]', 100), \
                (2, '明日', 'あした', 200, '[]', 200), \
                (2, '明日', 'みょうにち', 100, '[]', 200), \
                (2, '不器用', 'ぶきよう', 200, '[]', 300), \
                (2, '無器用', 'ぶきよう', -101, '[]', 300); \
            INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) VALUES \
                (3, '御辞儀', 'おじぎ', 12272), (3, 'お辞儀', 'おじぎ', 405782), \
                (3, '明日', 'あした', 1000), (3, '明日', 'みょうにち', 209173), \
                (3, '不器用', 'ふきよう', 14328), (3, '不器用', 'ぶきよう', 536048), \
                (3, '無器用', 'ぶきよう', 405782);";
        for stmt in sql.split(';').filter(|s| !s.trim().is_empty()) {
            sqlx::query(stmt).execute(k.pool()).await.unwrap();
        }
    }

    async fn rank_of(k: &Knowledge) -> Option<i64> {
        let (_, rows) = browse(k, None, None, false, false, 50, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "the fixture holds one ledger row");
        rows[0].rank
    }

    #[tokio::test]
    async fn a_word_is_as_common_as_the_corpus_spells_it() {
        let k = temp().await;
        corpus_that_spells_it_differently(&k).await;
        record_encounters(&k, &[enc("お辞儀", "オジギ", 1, 100.0)])
            .await
            .unwrap();

        assert_eq!(rank_of(&k).await, Some(12272));
    }

    /// The corpus reads it ふきよう where every dictionary says ぶきよう. One
    /// reading in the entry means there is no other word it could have counted.
    #[tokio::test]
    async fn a_word_the_corpus_reads_differently_is_still_that_word() {
        let k = temp().await;
        corpus_that_spells_it_differently(&k).await;
        record_encounters(&k, &[enc("不器用", "ブキヨウ", 1, 100.0)])
            .await
            .unwrap();

        assert_eq!(rank_of(&k).await, Some(14328));
    }

    #[tokio::test]
    async fn a_rare_reading_does_not_inherit_the_common_one() {
        let k = temp().await;
        corpus_that_spells_it_differently(&k).await;
        record_encounters(&k, &[enc("明日", "ミョウニチ", 1, 100.0)])
            .await
            .unwrap();

        assert_eq!(rank_of(&k).await, Some(209173));
    }

    #[tokio::test]
    async fn a_form_no_dictionary_resolves_keeps_the_rank_it_has() {
        let k = temp().await;
        corpus_that_spells_it_differently(&k).await;
        sqlx::query(
            "INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) \
             VALUES (3, 'ぬるぽ', 'ぬるぽ', 7)",
        )
        .execute(k.pool())
        .await
        .unwrap();
        record_encounters(&k, &[enc("ぬるぽ", "ヌルポ", 1, 100.0)])
            .await
            .unwrap();

        assert_eq!(rank_of(&k).await, Some(7));
    }

    /// A kana headword stores no reading, and matching the blank against a
    /// dictionary that always carries one found no siblings at all — which cost
    /// the ledger's commonest words their rank entirely: いる, この, また.
    #[tokio::test]
    async fn a_kana_headword_still_finds_its_kanji_spelling() {
        let k = temp().await;
        let sql = "\
            INSERT INTO dictionaries (id, title, source_path, role) VALUES \
                (2, 'lex', 'l.zip', 'reference'), (3, 'BCCWJ', 'b.zip', 'reference'); \
            INSERT INTO dictionary_entries (dictionary_id, term, reading, score, \
                                            definitions_json, sequence) VALUES \
                (2, '居る', 'いる', 200, '[]', 300), (2, 'いる', 'いる', 100, '[]', 300); \
            INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) \
                VALUES (3, '居る', 'いる', 13);";
        for stmt in sql.split(';').filter(|s| !s.trim().is_empty()) {
            sqlx::query(stmt).execute(k.pool()).await.unwrap();
        }
        record_encounters(&k, &[enc("いる", "イル", 1, 100.0)])
            .await
            .unwrap();

        assert_eq!(rank_of(&k).await, Some(13));
    }
}
