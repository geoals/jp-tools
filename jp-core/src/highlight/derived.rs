//! The dictionary-derived collections [`Pipeline`] is built from, cached whole
//! in `knowledge.db`.
//!
//! Each one is a pure function of the dictionaries, so none of them changes
//! until a dictionary does — but deriving them costs seconds and every tool paid
//! it on every start. The queries themselves are about one second; the rest is
//! turning 2.5M rows into Rust collections one row at a time. Indexing cannot
//! help, so the work has to stop being repeated.
//!
//! **`jp-dict` writes this and services only read it.** It owns every dictionary
//! mutation (`import`, `reimport`, `remove`, `set-role`, `priority`, `sync`), so
//! it is the one place that knows the inputs moved. A service that wrote the
//! cache would be three processes racing to author the same 40 MB.
//!
//! **A stale or absent cache is not a failure, only slower.** The fingerprint
//! stops matching and [`Derived::build`] runs the queries as before.
//!
//! [`Pipeline`]: super::Pipeline

use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::knowledge::dictionaries::{self as d, PreferredReading};

/// The parts the cache is stored in, one row each.
///
/// **Rows rather than one 50 MB blob, for two reasons.** Each is written as a
/// statement of its own, so `jp-dict` never holds the write lock for the whole
/// payload at once. And each decodes on a blocking thread of its own, which is
/// what takes the read off the critical path — decoding is 2.5M string
/// allocations and was the largest thing left in a start.
///
/// So the split is by *size*, not by meaning: the two wordhood sets are one
/// query but two rows, because together they are the longest pole.
const SECTIONS: [&str; 6] = [
    "wordhood_terms",
    "wordhood_readings",
    "master",
    "reader_ranks",
    "corpus_ranks",
    "arbitration",
];

/// The encoding *and* the derivation, versioned together, because a reader
/// cannot tell them apart: a blob written before a rule changed decodes cleanly
/// into collections this build would have derived differently. Bump it for
/// either.
const FORMAT: u32 = 2;

/// The corpus frequency list, found by title because it carries no role of its
/// own — see [`d::by_title`]. It arbitrates between spellings of one reading,
/// which is not what [`d::reader_frequency`] answers.
const CORPUS_FREQUENCY: &str = "BCCWJ";

/// Everything the pipeline needs out of the dictionaries, and nothing that comes
/// from anywhere else.
///
/// `mined_vocab` and `work_names` are deliberately absent: they are small, they
/// are not derived from a dictionary, and they change whenever a card is added
/// or a cast list is edited — caching them would make the fingerprint wrong for
/// reasons that have nothing to do with the dictionaries.
pub struct Derived {
    /// Every headword, and every reading, of the dictionaries whose listing
    /// makes a term a word — see [`d::wordhood_entries`].
    pub wordhood_terms: HashSet<String>,
    pub wordhood_readings: HashSet<String>,
    pub master_headwords: HashSet<String>,
    pub master_entries: Vec<(String, String)>,
    pub master_conjugatable: HashSet<String>,
    pub standard_entries: Vec<(String, String)>,
    /// Corpus ranks for the master headwords that share a reading with another.
    pub ambiguous_ranks: HashMap<(String, String), i64>,
    /// The reader-facing rank per spelling, every term the list carries.
    ///
    /// **The whole list, and shared rather than copied.** The tokenizer's
    /// short-kana guard reads an absent spelling as rare, so it needs all of it;
    /// the reader's underline wants the same numbers restricted to spellings a
    /// dictionary lists, which [`Highlighter::rank`] does at lookup time. An
    /// `Arc` because both hold it and a second copy of 443k ranks is 11 MB and
    /// 80 ms of startup for nothing.
    ///
    /// [`Highlighter::rank`]: super::Highlighter
    pub reader_ranks: std::sync::Arc<HashMap<String, i64>>,
    /// The corpus rank per spelling. One consumer, so no sharing to do.
    pub bccwj_ranks: HashMap<String, i64>,
    pub preferred_readings: HashMap<String, PreferredReading>,
}

/// What a cached payload has to agree with before it is used.
///
/// **Content, not a counter.** A generation number bumped by `jp-dict` would be
/// almost free and would be wrong the first time `knowledge.db` is edited by
/// hand, which happens. Row counts cost about 150 ms against the covering
/// indexes and catch every import, reimport and removal however it was made.
///
/// What they cannot catch is an edit that replaces a row in place. `jp-dict`
/// rebuilds whenever the fingerprint moves, so the way out of a wrong cache is
/// to change something it does see — or to delete the row.
pub async fn fingerprint(pool: &SqlitePool) -> Result<String, sqlx::Error> {
    let mut out = format!("v{FORMAT}");
    // `list_dictionaries` orders by (priority, id), so both the field and the
    // position move when a priority does.
    for dict in d::list_dictionaries(pool).await? {
        let mut counts = [0i64; 2];
        for (n, table) in ["dictionary_entries", "dictionary_frequency"]
            .into_iter()
            .enumerate()
        {
            counts[n] = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE dictionary_id = ?"
            ))
            .bind(dict.id)
            .fetch_one(pool)
            .await?;
        }
        out.push_str(&format!(
            " {}:{}:{}:{}:{}:{}",
            dict.id,
            dict.title,
            dict.role.as_str(),
            dict.priority,
            counts[0],
            counts[1],
        ));
    }
    Ok(out)
}

/// The collections, from the cache when it is current and from the dictionary
/// tables when it is not.
///
/// A miss keeps what it derived, so those seconds are paid once rather than on
/// every start. `jp-dict` fills the cache when a dictionary changes and is still
/// the only thing that fills it *before* a reader needs it, but nothing runs
/// `jp-dict` on the launcher's path — so without this, a machine whose
/// dictionaries were imported by an older build derives them again every time.
pub async fn load_or_build(pool: &SqlitePool) -> Result<Derived, sqlx::Error> {
    match Derived::load(pool).await {
        Ok(Some(derived)) => return Ok(derived),
        Ok(None) => tracing::info!(
            "no current derived cache — deriving the pipeline from the dictionary \
             tables, which takes seconds, and keeping it"
        ),
        Err(e) => tracing::warn!(error = %e, "cannot read the derived cache"),
    }
    // Read before the build rather than after it: a dictionary that changes
    // while the build runs then leaves the payload stamped with the older
    // fingerprint, and the next reader derives again instead of trusting a mix.
    let fingerprint = fingerprint(pool).await?;
    let derived = Derived::build(pool).await?;
    if let Err(e) = derived.store(pool, &fingerprint).await {
        tracing::warn!(error = %e, "cannot write the derived cache");
    }
    Ok(derived)
}

/// Bring the cache up to date, and say whether it had to be written.
///
/// `jp-dict`'s job after any change to the dictionaries. A run whose fingerprint
/// already matches writes nothing, so a `sync` with no new zip stays cheap.
pub async fn rebuild(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let want = fingerprint(pool).await?;
    if stored_fingerprint(pool).await? == Some(want.clone()) {
        return Ok(false);
    }
    Derived::build(pool).await?.store(pool, &want).await?;
    Ok(true)
}

/// The fingerprint every section agrees on, or `None` if any is missing or they
/// disagree — a half-written cache is no cache.
async fn stored_fingerprint(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, fingerprint FROM derived_cache")
        .fetch_all(pool)
        .await?;
    let Some(agreed) = SECTIONS
        .iter()
        .map(|want| rows.iter().find(|(name, _)| name == want).map(|(_, f)| f))
        .collect::<Option<Vec<&String>>>()
    else {
        return Ok(None);
    };
    let all_agree = agreed.windows(2).all(|w| w[0] == w[1]);
    Ok(agreed.first().filter(|_| all_agree).map(|f| f.to_string()))
}

impl Derived {
    /// Derive all of it from the dictionary tables — what every start did before
    /// the cache existed, and still does when it misses.
    async fn build(pool: &SqlitePool) -> Result<Derived, sqlx::Error> {
        let (wordhood_terms, wordhood_readings) = d::wordhood_entries(pool).await?;
        let master_entries = d::master_entries(pool).await?;
        Ok(Derived {
            wordhood_terms,
            wordhood_readings,
            master_headwords: d::master_headwords(pool).await?,
            master_conjugatable: d::master_conjugatable(pool).await?,
            standard_entries: d::standard_entries(pool).await?,
            ambiguous_ranks: ambiguous_ranks(pool, &master_entries).await?,
            reader_ranks: std::sync::Arc::new(
                ranks_of(pool, d::reader_frequency(pool).await?).await?,
            ),
            bccwj_ranks: ranks_of(pool, d::by_title(pool, CORPUS_FREQUENCY).await?).await?,
            preferred_readings: preferred_readings(pool).await?,
            master_entries,
        })
    }

    /// Read the cache back, if it holds these exact dictionaries.
    ///
    /// The fingerprint is computed while the rows are being read rather than
    /// before them: the two are independent queries and the fingerprint costs
    /// more than the read does, so serialising them would add its whole cost to
    /// the startup path. Then every section decodes at once, each on its own
    /// blocking thread.
    async fn load(pool: &SqlitePool) -> Result<Option<Derived>, sqlx::Error> {
        // Asked for by name rather than `SELECT *`: a payload left behind by an
        // older format is dead weight, and reading it costs its whole size.
        let sql = format!(
            "SELECT name, fingerprint, payload FROM derived_cache WHERE name IN ({})",
            ["?"; SECTIONS.len()].join(",")
        );
        let (want, rows) = tokio::try_join!(fingerprint(pool), async {
            let mut q = sqlx::query_as::<_, (String, String, Vec<u8>)>(&sql);
            for section in SECTIONS {
                q = q.bind(section);
            }
            q.fetch_all(pool).await
        })?;

        let mut payloads: HashMap<String, Vec<u8>> = HashMap::new();
        for (name, stored, payload) in rows {
            if stored == want {
                payloads.insert(name, payload);
            }
        }
        if SECTIONS.iter().any(|s| !payloads.contains_key(*s)) {
            return Ok(None);
        }
        let mut take = |name: &str| payloads.remove(name).unwrap_or_default();
        let (terms, readings, master, reader, corpus, arbitration) = (
            take("wordhood_terms"),
            take("wordhood_readings"),
            take("master"),
            take("reader_ranks"),
            take("corpus_ranks"),
            take("arbitration"),
        );

        // Decoding is half a second of pure CPU altogether, which must neither
        // sit on the runtime a server is polling requests on nor happen one
        // section after another.
        let decoded = tokio::try_join!(
            spawn(move || decode_set(&terms)),
            spawn(move || decode_set(&readings)),
            spawn(move || decode_master(&master)),
            spawn(move || decode_ranks(&reader)),
            spawn(move || decode_ranks(&corpus)),
            spawn(move || decode_arbitration(&arbitration)),
        );
        let Ok((
            Some(wordhood_terms),
            Some(wordhood_readings),
            Some(master),
            Some(reader_ranks),
            Some(bccwj_ranks),
            Some(arbitration),
        )) = decoded
        else {
            tracing::warn!("the derived cache does not decode — rebuilding it");
            return Ok(None);
        };
        let (master_headwords, master_entries, master_conjugatable, standard_entries) = master;
        let (ambiguous_ranks, preferred_readings) = arbitration;
        Ok(Some(Derived {
            wordhood_terms,
            wordhood_readings,
            master_headwords,
            master_entries,
            master_conjugatable,
            standard_entries,
            ambiguous_ranks,
            reader_ranks: std::sync::Arc::new(reader_ranks),
            bccwj_ranks,
            preferred_readings,
        }))
    }

    async fn store(&self, pool: &SqlitePool, fingerprint: &str) -> Result<(), sqlx::Error> {
        let built = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        // One statement per section, so a 50 MB write is never one transaction:
        // SQLite takes a single write lock per database and a source posting a
        // line waits behind whatever holds it.
        for (name, payload) in self.encode() {
            sqlx::query(
                "INSERT INTO derived_cache (name, fingerprint, payload, built_ts) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT(name) DO UPDATE SET fingerprint = excluded.fingerprint, \
                     payload = excluded.payload, built_ts = excluded.built_ts",
            )
            .bind(name)
            .bind(fingerprint)
            .bind(payload)
            .bind(built)
            .execute(pool)
            .await?;
        }
        // A payload an older format wrote under a name this one does not use
        // would sit there forever, tens of megabytes of it.
        let sql = format!(
            "DELETE FROM derived_cache WHERE name NOT IN ({})",
            ["?"; SECTIONS.len()].join(",")
        );
        let mut sweep = sqlx::query(&sql);
        for section in SECTIONS {
            sweep = sweep.bind(section);
        }
        sweep.execute(pool).await?;
        Ok(())
    }
}

/// Corpus ranks for the master headwords that share a reading with another, so
/// the tokenizer can name a word written in kana (うかがう → 伺う over 窺う).
///
/// **Stays on the corpus list** where the reader-facing ranks do not: this asks
/// which spelling of one reading is the commoner one, and a list carrying
/// kana-only rows would answer it with the reading's own rank. Not being loaded
/// is not an error — ambiguous readings are then left unresolved.
async fn ambiguous_ranks(
    pool: &SqlitePool,
    master_entries: &[(String, String)],
) -> Result<HashMap<(String, String), i64>, sqlx::Error> {
    let Some(corpus) = d::by_title(pool, CORPUS_FREQUENCY).await? else {
        return Ok(HashMap::new());
    };
    let terms = crate::tokenize::ambiguous_headwords(master_entries);
    d::frequency_ranks(pool, corpus.id, &terms).await
}

/// One frequency list's best rank per spelling. Empty when that list is not
/// loaded, which is never an error — the features built on it simply go quiet.
///
/// **Keyed by spelling alone, best rank wins** — the same question
/// `lookup_frequency` puts for the popup. Keying on `(spelling, reading)`
/// instead made the underline and the popup disagree about the same word: the
/// popup printed 4,259 for 近付ける while the span carried nothing.
async fn ranks_of(
    pool: &SqlitePool,
    dict: Option<d::Dictionary>,
) -> Result<HashMap<String, i64>, sqlx::Error> {
    let Some(dict) = dict else {
        return Ok(HashMap::new());
    };
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT term, MIN(frequency) FROM dictionary_frequency \
         WHERE dictionary_id = ? GROUP BY term",
    )
    .bind(dict.id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Which reading to believe for a headword the master lists several ways — see
/// [`d::preferred_readings`], which decides it. Three dictionaries have to be
/// present; missing any of them leaves the reading to Sudachi.
async fn preferred_readings(
    pool: &SqlitePool,
) -> Result<HashMap<String, PreferredReading>, sqlx::Error> {
    let (Some(master), Some(popularity), Some(corpus)) = (
        d::master(pool).await?,
        d::by_title(pool, "Jitendex").await?,
        d::by_title(pool, CORPUS_FREQUENCY).await?,
    ) else {
        return Ok(HashMap::new());
    };
    d::preferred_readings(pool, master.id, popularity.id, corpus.id).await
}

/// What the master dictionary and the standard ones beside it contribute, as one
/// section: four collections out of three queries, all small next to the ranks.
type MasterSection = (
    HashSet<String>,
    Vec<(String, String)>,
    HashSet<String>,
    Vec<(String, String)>,
);

/// The two tables that arbitrate rather than describe — which spelling of a
/// shared reading, and which reading of a shared spelling. A few tens of
/// thousands of rows between them.
type ArbitrationSection = (
    HashMap<(String, String), i64>,
    HashMap<String, PreferredReading>,
);

fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> tokio::task::JoinHandle<T> {
    tokio::task::spawn_blocking(work)
}

/// A flat little-endian encoding: a `u32` count, then each string as a `u32`
/// byte length and its bytes, and each rank as an `i64`.
///
/// Not JSON. This is read on the path that decides how long the reader waits
/// for the first tinted line, and a serde pass over 2M strings is the cost the
/// cache exists to remove.
impl Derived {
    fn encode(&self) -> Vec<(&'static str, Vec<u8>)> {
        let mut master = Vec::with_capacity(12 << 20);
        put_set(&mut master, &self.master_headwords);
        put_pairs(&mut master, &self.master_entries);
        put_set(&mut master, &self.master_conjugatable);
        put_pairs(&mut master, &self.standard_entries);

        let mut arbitration = Vec::with_capacity(4 << 20);
        put_count(&mut arbitration, self.ambiguous_ranks.len());
        for ((term, reading), rank) in &self.ambiguous_ranks {
            put_str(&mut arbitration, term);
            put_str(&mut arbitration, reading);
            put_int(&mut arbitration, *rank);
        }
        put_count(&mut arbitration, self.preferred_readings.len());
        for (term, preference) in &self.preferred_readings {
            put_str(&mut arbitration, term);
            put_str(&mut arbitration, &preference.preferred);
            put_set(&mut arbitration, &preference.acceptable);
        }

        vec![
            ("wordhood_terms", encode_set(&self.wordhood_terms)),
            ("wordhood_readings", encode_set(&self.wordhood_readings)),
            ("master", master),
            ("reader_ranks", encode_ranks(&self.reader_ranks)),
            ("corpus_ranks", encode_ranks(&self.bccwj_ranks)),
            ("arbitration", arbitration),
        ]
    }
}

fn encode_set(set: &HashSet<String>) -> Vec<u8> {
    let mut out = Vec::with_capacity(set.len() * 16);
    put_set(&mut out, set);
    out
}

fn encode_ranks(ranks: &HashMap<String, i64>) -> Vec<u8> {
    let mut out = Vec::with_capacity(ranks.len() * 24);
    put_count(&mut out, ranks.len());
    for (term, rank) in ranks {
        put_str(&mut out, term);
        put_int(&mut out, *rank);
    }
    out
}

/// Each `decode_*` returns `None` for a section that is truncated, not UTF-8,
/// or written by another format. Every read is bounds-checked and anything left
/// over is a rejection, so a corrupt cache costs a rebuild rather than a panic
/// on the path that brings the reader up.
fn decode_set(payload: &[u8]) -> Option<HashSet<String>> {
    let mut r = Reader::new(payload);
    let set = r.set()?;
    r.done().then_some(set)
}

fn decode_ranks(payload: &[u8]) -> Option<HashMap<String, i64>> {
    let mut r = Reader::new(payload);
    let ranks = r.ranks()?;
    r.done().then_some(ranks)
}

fn decode_master(payload: &[u8]) -> Option<MasterSection> {
    let mut r = Reader::new(payload);
    let headwords = r.set()?;
    let entries = r.pairs()?;
    let conjugatable = r.set()?;
    let standard = r.pairs()?;
    r.done()
        .then_some((headwords, entries, conjugatable, standard))
}

fn decode_arbitration(payload: &[u8]) -> Option<ArbitrationSection> {
    let mut r = Reader::new(payload);
    let n = r.count(16)?;
    let mut ambiguous = HashMap::with_capacity(n);
    for _ in 0..n {
        let key = (r.str()?.to_string(), r.str()?.to_string());
        ambiguous.insert(key, r.int()?);
    }
    let n = r.count(12)?;
    let mut preferred = HashMap::with_capacity(n);
    for _ in 0..n {
        let term = r.str()?.to_string();
        let reading = r.str()?.to_string();
        preferred.insert(
            term,
            PreferredReading {
                preferred: reading,
                acceptable: r.set()?,
            },
        );
    }
    r.done().then_some((ambiguous, preferred))
}

fn put_count(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_le_bytes());
}

fn put_set(out: &mut Vec<u8>, set: &HashSet<String>) {
    put_count(out, set.len());
    for term in set {
        put_str(out, term);
    }
}

fn put_pairs(out: &mut Vec<u8>, pairs: &[(String, String)]) {
    put_count(out, pairs.len());
    for (term, reading) in pairs {
        put_str(out, term);
        put_str(out, reading);
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_count(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

fn put_int(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, at: 0 }
    }

    fn done(&self) -> bool {
        self.at == self.buf.len()
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.buf.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(slice)
    }

    /// A count, and the capacity to reserve for it. `least` is the smallest
    /// number of bytes one element can occupy, so a corrupt count cannot ask for
    /// an allocation larger than the rest of the payload could fill.
    fn count(&mut self, least: usize) -> Option<usize> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        if n > (self.buf.len() - self.at) / least {
            return None;
        }
        Some(n)
    }

    fn str(&mut self) -> Option<&'a str> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        std::str::from_utf8(self.take(n)?).ok()
    }

    fn int(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn set(&mut self) -> Option<HashSet<String>> {
        let n = self.count(4)?;
        let mut set = HashSet::with_capacity(n);
        for _ in 0..n {
            set.insert(self.str()?.to_string());
        }
        Some(set)
    }

    fn pairs(&mut self) -> Option<Vec<(String, String)>> {
        let n = self.count(8)?;
        let mut pairs = Vec::with_capacity(n);
        for _ in 0..n {
            pairs.push((self.str()?.to_string(), self.str()?.to_string()));
        }
        Some(pairs)
    }

    fn ranks(&mut self) -> Option<HashMap<String, i64>> {
        let n = self.count(12)?;
        let mut ranks = HashMap::with_capacity(n);
        for _ in 0..n {
            let term = self.str()?.to_string();
            ranks.insert(term, self.int()?);
        }
        Some(ranks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::Knowledge;

    fn sample() -> Derived {
        Derived {
            wordhood_terms: ["景気づけ".to_string(), "を".to_string()]
                .into_iter()
                .collect(),
            wordhood_readings: ["むわむわ".to_string()].into_iter().collect(),
            master_headwords: ["私".to_string()].into_iter().collect(),
            master_entries: vec![
                ("私".to_string(), "わたし".to_string()),
                ("私".to_string(), "わたくし".to_string()),
            ],
            master_conjugatable: ["許す".to_string()].into_iter().collect(),
            standard_entries: vec![("意味ありげ".to_string(), "いみありげ".to_string())],
            ambiguous_ranks: [(("伺う".to_string(), "うかがう".to_string()), 4259)]
                .into_iter()
                .collect(),
            reader_ranks: std::sync::Arc::new([("私".to_string(), 12i64)].into_iter().collect()),
            bccwj_ranks: [("私".to_string(), 182i64)].into_iter().collect(),
            preferred_readings: [(
                "私".to_string(),
                PreferredReading {
                    preferred: "わたし".to_string(),
                    acceptable: ["わたし".to_string(), "あたし".to_string()]
                        .into_iter()
                        .collect(),
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    /// Decode every section back, the way [`Derived::load`] assembles them.
    fn round_trip(derived: &Derived) -> Derived {
        let mut by_name: HashMap<&str, Vec<u8>> = derived.encode().into_iter().collect();
        let (master_headwords, master_entries, master_conjugatable, standard_entries) =
            decode_master(&by_name.remove("master").unwrap()).expect("master");
        let (ambiguous_ranks, preferred_readings) =
            decode_arbitration(&by_name.remove("arbitration").unwrap()).expect("arbitration");
        Derived {
            wordhood_terms: decode_set(&by_name.remove("wordhood_terms").unwrap()).expect("terms"),
            wordhood_readings: decode_set(&by_name.remove("wordhood_readings").unwrap())
                .expect("readings"),
            master_headwords,
            master_entries,
            master_conjugatable,
            standard_entries,
            ambiguous_ranks,
            reader_ranks: std::sync::Arc::new(
                decode_ranks(&by_name.remove("reader_ranks").unwrap()).expect("reader ranks"),
            ),
            bccwj_ranks: decode_ranks(&by_name.remove("corpus_ranks").unwrap())
                .expect("corpus ranks"),
            preferred_readings,
        }
    }

    #[test]
    fn every_collection_survives_the_round_trip() {
        let want = sample();
        let got = round_trip(&want);
        assert_eq!(got.wordhood_terms, want.wordhood_terms);
        assert_eq!(got.wordhood_readings, want.wordhood_readings);
        assert_eq!(got.master_headwords, want.master_headwords);
        assert_eq!(got.master_entries, want.master_entries);
        assert_eq!(got.master_conjugatable, want.master_conjugatable);
        assert_eq!(got.standard_entries, want.standard_entries);
        assert_eq!(got.ambiguous_ranks, want.ambiguous_ranks);
        assert_eq!(got.reader_ranks, want.reader_ranks);
        assert_eq!(got.bccwj_ranks, want.bccwj_ranks);
        let preference = got.preferred_readings.get("私").expect("私");
        assert_eq!(preference.preferred, "わたし");
        assert!(preference.acceptable.contains("あたし"));
    }

    /// Every section must name itself, or `load` silently drops one and the
    /// fingerprint check passes over a cache that is missing a collection.
    #[test]
    fn the_sections_written_are_the_sections_expected() {
        let written: HashSet<&str> = sample().encode().into_iter().map(|(n, _)| n).collect();
        assert_eq!(written, SECTIONS.into_iter().collect::<HashSet<&str>>());
    }

    /// A truncated or overlong section must cost a rebuild, never a panic: this
    /// decodes on the path that brings the reader up.
    #[test]
    fn a_damaged_section_decodes_to_nothing() {
        for (name, whole) in sample().encode() {
            let decode: fn(&[u8]) -> bool = match name {
                "wordhood_terms" | "wordhood_readings" => |b| decode_set(b).is_some(),
                "master" => |b| decode_master(b).is_some(),
                "reader_ranks" | "corpus_ranks" => |b| decode_ranks(b).is_some(),
                _ => |b| decode_arbitration(b).is_some(),
            };
            assert!(decode(&whole), "{name} decodes as written");
            for cut in [0, 1, 3, whole.len() / 2, whole.len() - 1] {
                assert!(!decode(&whole[..cut]), "{name} truncated at {cut}");
            }
            let mut extra = whole.clone();
            extra.push(0);
            assert!(!decode(&extra), "{name} with trailing bytes");
            // A count that claims more than the section could possibly hold.
            let mut lying = whole.clone();
            lying[..4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(!decode(&lying), "{name} with an impossible count");
        }
    }

    #[tokio::test]
    async fn an_empty_database_caches_and_reads_back_empty() {
        let k = Knowledge::temp().await;
        let pool = k.pool();
        // Nothing is cached yet, so the first build is from the (empty) rows.
        assert!(Derived::load(pool).await.unwrap().is_none());
        assert!(rebuild(pool).await.unwrap(), "the first run writes");
        assert!(
            !rebuild(pool).await.unwrap(),
            "the second has nothing to do"
        );
        assert!(Derived::load(pool).await.unwrap().is_some());
    }

    /// A start that had to derive leaves the cache current, so the next one
    /// reads it — and `jp-dict` then agrees there is nothing to write, which is
    /// the two paths stamping the same fingerprint.
    #[tokio::test]
    async fn a_reader_that_derives_keeps_what_it_derived() {
        let k = Knowledge::temp().await;
        let pool = k.pool();
        assert!(Derived::load(pool).await.unwrap().is_none());
        load_or_build(pool).await.unwrap();
        assert!(Derived::load(pool).await.unwrap().is_some());
        assert!(!rebuild(pool).await.unwrap(), "nothing left for jp-dict");
    }

    /// **The claim the whole cache rests on**: what it hands back is what the
    /// queries would have derived. Run this against a real `knowledge.db` after
    /// touching the codec or any of the queries [`Derived::build`] makes.
    ///
    /// The two `Vec` fields are compared in order and that matters:
    /// `with_master_readings` and `with_standard` fill their maps with
    /// `entry().or_insert()`, so a reordered list gives a headword a different
    /// reading and the tokenizer answers differently.
    #[tokio::test]
    async fn the_cache_hands_back_what_the_queries_derive() {
        let k = Knowledge::temp().await;
        let pool = k.pool();
        for (title, role) in [
            ("Sankoku", d::Role::Master),
            ("Jitendex", d::Role::Reference),
            ("BCCWJ", d::Role::Frequency),
            ("Jiten", d::Role::Frequency),
            ("明鏡", d::Role::Standard),
        ] {
            sqlx::query("INSERT INTO dictionaries (title, source_path, role) VALUES (?, ?, ?)")
                .bind(title)
                .bind(format!("/x/{title}.zip"))
                .bind(role.as_str())
                .execute(pool)
                .await
                .unwrap();
        }
        // 私 is listed three ways so `preferred_readings` has something to
        // decide, and 伺う/窺う share a reading so `ambiguous_ranks` does.
        for (dict, term, reading, score) in [
            (1i64, "私", "わたし", 0i64),
            (1, "私", "わたくし", 0),
            (1, "私", "あたし", 0),
            (1, "伺う", "うかがう", 0),
            (1, "窺う", "うかがう", 0),
            (1, "許す", "ゆるす", 0),
            (2, "私", "わたし", 200),
            (2, "私", "あたし", 200),
            (2, "私", "わたくし", 0),
            (5, "意味ありげ", "いみありげ", 0),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_entries \
                 (dictionary_id, term, reading, score, definitions_json, rules) \
                 VALUES (?, ?, ?, ?, '[]', 'v5')",
            )
            .bind(dict)
            .bind(term)
            .bind(reading)
            .bind(score)
            .execute(pool)
            .await
            .unwrap();
        }
        for (dict, term, reading, rank) in [
            (3i64, "私", "わたし", 182i64),
            (3, "私", "わたくし", 47),
            (3, "私", "あたし", 678),
            (3, "伺う", "うかがう", 4259),
            (3, "窺う", "うかがう", 91000),
            (4, "私", "わたし", 12),
            (4, "意味ありげ", "いみありげ", 30000),
        ] {
            sqlx::query(
                "INSERT INTO dictionary_frequency (dictionary_id, term, reading, frequency) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(dict)
            .bind(term)
            .bind(reading)
            .bind(rank)
            .execute(pool)
            .await
            .unwrap();
        }

        rebuild(pool).await.unwrap();
        let cached = Derived::load(pool).await.unwrap().expect("just written");
        let fresh = Derived::build(pool).await.unwrap();

        assert_eq!(cached.wordhood_terms, fresh.wordhood_terms);
        assert_eq!(cached.wordhood_readings, fresh.wordhood_readings);
        assert_eq!(cached.master_headwords, fresh.master_headwords);
        assert_eq!(cached.master_entries, fresh.master_entries);
        assert_eq!(cached.master_conjugatable, fresh.master_conjugatable);
        assert_eq!(cached.standard_entries, fresh.standard_entries);
        assert_eq!(cached.ambiguous_ranks, fresh.ambiguous_ranks);
        assert_eq!(cached.reader_ranks, fresh.reader_ranks);
        assert_eq!(cached.bccwj_ranks, fresh.bccwj_ranks);
        assert_eq!(
            cached.preferred_readings.len(),
            fresh.preferred_readings.len()
        );
        for (term, want) in &fresh.preferred_readings {
            let got = cached.preferred_readings.get(term).expect(term);
            assert_eq!(got.preferred, want.preferred);
            assert_eq!(got.acceptable, want.acceptable);
        }

        // And the fixture has to be rich enough that the comparison means
        // something — an all-empty cache would pass every assertion above.
        assert!(!cached.wordhood_terms.is_empty());
        assert!(!cached.master_entries.is_empty());
        assert!(!cached.standard_entries.is_empty());
        assert!(!cached.ambiguous_ranks.is_empty());
        assert!(!cached.reader_ranks.is_empty());
        assert!(!cached.bccwj_ranks.is_empty());
        assert!(!cached.master_conjugatable.is_empty());
        assert!(
            cached.preferred_readings.contains_key("私"),
            "私 is わたし, わたくし and あたし — the case preferences exist for"
        );
    }

    /// The fingerprint has to move when a dictionary does, or a cache built
    /// against the old set keeps answering.
    #[tokio::test]
    async fn the_fingerprint_follows_the_dictionaries() {
        let k = Knowledge::temp().await;
        let pool = k.pool();
        let empty = fingerprint(pool).await.unwrap();

        sqlx::query("INSERT INTO dictionaries (title, source_path) VALUES ('Sankoku', '/x/s.zip')")
            .execute(pool)
            .await
            .unwrap();
        let imported = fingerprint(pool).await.unwrap();
        assert_ne!(empty, imported, "a new dictionary");

        rebuild(pool).await.unwrap();
        assert!(Derived::load(pool).await.unwrap().is_some());

        sqlx::query(
            "INSERT INTO dictionary_entries (dictionary_id, term, reading, definitions_json) \
             VALUES (1, '私', 'わたし', '[]')",
        )
        .execute(pool)
        .await
        .unwrap();
        assert_ne!(imported, fingerprint(pool).await.unwrap(), "a new entry");
        assert!(
            Derived::load(pool).await.unwrap().is_none(),
            "the cache stops answering the moment its inputs move"
        );

        d::set_role(pool, 1, d::Role::Master).await.unwrap();
        let as_master = fingerprint(pool).await.unwrap();
        d::set_role(pool, 1, d::Role::Reference).await.unwrap();
        assert_ne!(
            as_master,
            fingerprint(pool).await.unwrap(),
            "a role decides what a dictionary may answer, so it is an input"
        );
    }
}
