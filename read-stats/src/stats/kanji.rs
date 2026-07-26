//! Every kanji ever read, and what is known about each one.
//!
//! One pass over the raw text of the line stream produces the whole kanji tab:
//! the grid, the grade coverage, the discovery curve, the hardest-kanji ranking
//! and the per-work fingerprints are all re-slices of the same per-kanji rows.
//! They are computed together because they must agree — a kanji the grid tints
//! as familiar cannot be missing from the grade coverage it counts toward.
//!
//! Three side inputs join by *containment*, which is the only join available
//! without re-tokenizing: a lookup or a mined card counts toward a kanji when
//! its term contains that kanji. It over-credits compounds (looking up 邂逅
//! marks both), and that is the right bias here — both kanji genuinely were
//! part of what stopped you.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::NaiveDate;
use jp_core::text::kanji::{self, JOYO_GRADES};
use serde::Serialize;

use super::day::date_key;

/// Encounters before a kanji counts as *solid* rather than merely met. Ten is
/// where recognition stops being a coin flip in practice; it is also roughly
/// the knee of the frequency curve, so the number separates the kanji you read
/// through from the long tail you decode.
pub const SOLID_ENCOUNTERS: i64 = 10;

/// A kanji must appear this often in a work before its fingerprint ratio means
/// anything — below it, one scene of a character's name outranks everything.
const FINGERPRINT_FLOOR: i64 = 5;

/// Encounters before a lookup rate is a rate rather than an accident of one
/// hard sentence. One lookup on one sighting says nothing; six sightings is
/// enough for the ratio to have a denominator worth dividing by.
pub const OUTLIER_ENCOUNTERS: i64 = 6;

/// How far above your own baseline a kanji's lookup rate must sit before it
/// counts as one you are struggling with. The baseline is not zero — every
/// kanji costs *some* lookups — so the question is never "did you look it up"
/// but "did you look it up out of all proportion to how often you met it".
///
/// Eight is calibrated to the grid rather than to theory: it is the multiple
/// at which the ring marks a few dozen kanji out of a couple of thousand, and
/// a mark you can count is a mark you can act on. Three times marked 170 of
/// 1740, which is a wall of red saying nothing in particular. Because the bar
/// is a multiple of your own average and not a fixed rate, it stays that
/// selective as the reading gets easier.
pub const OUTLIER_MULTIPLE: f64 = 8.0;

/// How many example words and fingerprint entries each row carries.
const WORDS_PER_KANJI: usize = 5;
const FINGERPRINT_LEN: usize = 12;

/// A line reduced to what the kanji pass needs: when, what, and where from.
#[derive(Debug, Clone)]
pub struct KanjiLine {
    pub ts: f64,
    pub text: String,
    pub work: Option<String>,
}

/// A term the reader looked up, and how often.
#[derive(Debug, Clone)]
pub struct TermTimes {
    pub term: String,
    pub times: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KanjiRow {
    pub kanji: String,
    pub count: i64,
    pub first_ts: f64,
    pub last_ts: f64,
    /// Distinct days it was read on — a kanji met 40 times in one scene is not
    /// the same as one met 40 times across three weeks.
    pub days: usize,
    pub grade: Option<u8>,
    pub strokes: Option<u8>,
    pub freq: Option<u16>,
    /// Rank in BCCWJ, 1 = commonest kanji in written Japanese. `None` means a
    /// hundred million words of balanced text never used it.
    pub bccwj_rank: Option<u16>,
    /// Its share of BCCWJ, per million kanji — the yardstick your own count is
    /// read against.
    pub bccwj_per_million: Option<f64>,
    pub gloss: &'static str,
    /// Lookup *events* on terms containing this kanji.
    pub lookups: i64,
    /// Looked up far out of proportion to how often it was read — see
    /// [`OUTLIER_MULTIPLE`]. This is the only "you are struggling with this"
    /// verdict the tab makes; a single lookup is not one.
    pub struggling: bool,
    /// The target word of a card in the deck contains it.
    pub mined: bool,
    /// The work it was read in most.
    pub top_work: Option<String>,
    /// The most-read words containing it, commonest first.
    pub words: Vec<String>,
}

impl KanjiRow {
    /// Lookups per encounter — what this kanji costs to read past. The ranking
    /// is only meaningful above a floor of encounters, which the caller of the
    /// "hardest" list applies rather than this being pre-filtered away.
    pub fn lookup_rate(&self) -> f64 {
        self.lookups as f64 / self.count.max(1) as f64
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GradeCoverage {
    /// 1-6 primary, 8 the jōyō remainder, 0 the kanji outside any grade.
    pub grade: u8,
    pub total: usize,
    pub met: usize,
    pub solid: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct KanjiDay {
    pub date: String,
    /// Kanji met for the first time on this day.
    pub new: usize,
    /// Unique kanji known by the end of it.
    pub cumulative: usize,
    pub encounters: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkFingerprint {
    pub work: String,
    pub encounters: i64,
    pub unique: usize,
    /// Kanji this work leans on far harder than the rest of the corpus does,
    /// as `(kanji, count, ratio)` — ratio 3.0 means three times its usual share.
    pub distinctive: Vec<(String, i64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KanjiStats {
    pub kanji: Vec<KanjiRow>,
    pub grades: Vec<GradeCoverage>,
    pub days: Vec<KanjiDay>,
    pub works: Vec<WorkFingerprint>,
    pub total_encounters: i64,
    /// Lookups per kanji read, across everything — what a kanji costs on
    /// average. [`KanjiRow::struggling`] is a multiple of this, so it moves
    /// with the reader: as the baseline falls, the bar for "struggling" falls
    /// with it.
    pub baseline_lookup_rate: f64,
}

/// Everything accumulated for one kanji while the pass runs. Split from
/// [`KanjiRow`] because the maps are working state, not output.
#[derive(Default)]
struct Acc {
    count: i64,
    first_ts: f64,
    last_ts: f64,
    days: HashSet<NaiveDate>,
    works: HashMap<String, i64>,
}

/// The whole kanji tab, from the line stream and the three side inputs.
///
/// `words` is `(lemma, count)` from `word_days` — already the tokenizer's
/// verdict on what the text contained, so example words cost no tokenization
/// here.
pub fn aggregate_kanji(
    lines: &[KanjiLine],
    lookups: &[TermTimes],
    mined_vocab: &[String],
    words: &[(String, i64)],
    rollover_hour: i64,
    tz_offset_secs: i64,
) -> KanjiStats {
    let mut acc: HashMap<char, Acc> = HashMap::new();
    let mut per_work: HashMap<String, HashMap<char, i64>> = HashMap::new();
    // First-encounter day per kanji, which is what the discovery curve counts.
    let mut first_day: HashMap<char, NaiveDate> = HashMap::new();
    let mut day_encounters: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut total = 0i64;

    for line in lines {
        let day = date_key(line.ts, rollover_hour, tz_offset_secs);
        for c in line.text.chars().filter(|&c| kanji::is_kanji(c)) {
            let e = acc.entry(c).or_insert_with(|| Acc {
                first_ts: line.ts,
                ..Default::default()
            });
            e.count += 1;
            e.last_ts = line.ts;
            e.days.insert(day);
            if let Some(w) = &line.work {
                *e.works.entry(w.clone()).or_default() += 1;
                *per_work.entry(w.clone()).or_default().entry(c).or_default() += 1;
            }
            first_day.entry(c).or_insert(day);
            *day_encounters.entry(day).or_default() += 1;
            total += 1;
        }
    }

    // Containment joins. Each distinct kanji in a term is credited once per
    // lookup event, so a term looked up five times weighs five — the cost is
    // per encounter with the wall, not per word in the dictionary.
    let mut lookup_hits: HashMap<char, i64> = HashMap::new();
    for l in lookups {
        for c in distinct_kanji(&l.term) {
            *lookup_hits.entry(c).or_default() += l.times;
        }
    }
    let mut mined: HashSet<char> = HashSet::new();
    for v in mined_vocab {
        mined.extend(distinct_kanji(v));
    }

    // Example words: the commonest read words containing each kanji.
    let mut examples: HashMap<char, Vec<(i64, &str)>> = HashMap::new();
    for (lemma, count) in words {
        for c in distinct_kanji(lemma) {
            examples
                .entry(c)
                .or_default()
                .push((*count, lemma.as_str()));
        }
    }

    let mut rows: Vec<KanjiRow> = acc
        .into_iter()
        .map(|(c, a)| {
            let info = kanji::info(c);
            let bccwj = kanji::bccwj(c);
            let mut words: Vec<(i64, &str)> = examples.remove(&c).unwrap_or_default();
            words.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(y.1)));
            KanjiRow {
                kanji: c.to_string(),
                count: a.count,
                first_ts: a.first_ts,
                last_ts: a.last_ts,
                days: a.days.len(),
                grade: info.and_then(|i| i.grade),
                strokes: info.and_then(|i| i.strokes),
                freq: info.and_then(|i| i.freq),
                bccwj_rank: bccwj.map(|b| b.rank),
                bccwj_per_million: bccwj.map(|b| b.per_million()),
                gloss: info.map(|i| i.gloss).unwrap_or(""),
                lookups: lookup_hits.get(&c).copied().unwrap_or(0),
                // Filled in below, once the baseline it is measured against is
                // known.
                struggling: false,
                mined: mined.contains(&c),
                top_work: a.works.into_iter().max_by_key(|(_, n)| *n).map(|(w, _)| w),
                words: words
                    .into_iter()
                    .take(WORDS_PER_KANJI)
                    .map(|(_, w)| w.to_string())
                    .collect(),
            }
        })
        .collect();
    // Commonest first: the grid's default order, and the order the coverage
    // curve integrates in.
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.kanji.cmp(&b.kanji)));

    // The baseline is your own average cost per kanji read, over everything.
    // Marking outliers against it rather than against a fixed rate means the
    // mark keeps meaning the same thing as the reading gets easier.
    let total_lookups: i64 = rows.iter().map(|r| r.lookups).sum();
    let baseline = total_lookups as f64 / total.max(1) as f64;
    let bar = baseline * OUTLIER_MULTIPLE;
    for r in rows.iter_mut() {
        r.struggling = r.count >= OUTLIER_ENCOUNTERS && r.lookup_rate() > bar;
    }

    KanjiStats {
        grades: grade_coverage(&rows),
        days: discovery_days(&first_day, &day_encounters),
        works: fingerprints(&per_work),
        total_encounters: total,
        baseline_lookup_rate: baseline,
        kanji: rows,
    }
}

fn distinct_kanji(s: &str) -> impl Iterator<Item = char> {
    let mut seen = HashSet::new();
    s.chars()
        .filter(|&c| kanji::is_kanji(c))
        .filter(move |&c| seen.insert(c))
        .collect::<Vec<_>>()
        .into_iter()
}

/// Coverage per jōyō grade, plus grade 0 for everything read that no grade
/// teaches. Grade 0's `total` is the count met rather than a set size — there
/// is no denominator for "kanji Japanese schools never get to".
fn grade_coverage(rows: &[KanjiRow]) -> Vec<GradeCoverage> {
    let mut out: Vec<GradeCoverage> = JOYO_GRADES
        .iter()
        .map(|&g| GradeCoverage {
            grade: g,
            total: kanji::grade_size(g),
            met: 0,
            solid: 0,
        })
        .collect();
    out.push(GradeCoverage {
        grade: 0,
        total: 0,
        met: 0,
        solid: 0,
    });
    for r in rows {
        let grade = r.grade.filter(|g| JOYO_GRADES.contains(g)).unwrap_or(0);
        if let Some(c) = out.iter_mut().find(|c| c.grade == grade) {
            c.met += 1;
            if r.count >= SOLID_ENCOUNTERS {
                c.solid += 1;
            }
            if grade == 0 {
                c.total += 1;
            }
        }
    }
    out
}

fn discovery_days(
    first_day: &HashMap<char, NaiveDate>,
    day_encounters: &BTreeMap<NaiveDate, i64>,
) -> Vec<KanjiDay> {
    let mut new_per_day: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    for day in first_day.values() {
        *new_per_day.entry(*day).or_default() += 1;
    }
    let mut cumulative = 0;
    day_encounters
        .iter()
        .map(|(date, encounters)| {
            let new = new_per_day.get(date).copied().unwrap_or(0);
            cumulative += new;
            KanjiDay {
                date: date.to_string(),
                new,
                cumulative,
                encounters: *encounters,
            }
        })
        .collect()
}

/// For each work, the kanji whose share of that work's text most exceeds their
/// share of everything else read. A plain frequency list would just be 人事言
/// for every work; the ratio against the rest of the corpus is what makes two
/// VNs look different.
fn fingerprints(per_work: &HashMap<String, HashMap<char, i64>>) -> Vec<WorkFingerprint> {
    let mut totals: HashMap<char, i64> = HashMap::new();
    let mut grand = 0i64;
    for counts in per_work.values() {
        for (c, n) in counts {
            *totals.entry(*c).or_default() += n;
            grand += n;
        }
    }

    let mut out: Vec<WorkFingerprint> = per_work
        .iter()
        .map(|(work, counts)| {
            let here: i64 = counts.values().sum();
            let elsewhere = (grand - here).max(1);
            let mut ranked: Vec<(String, i64, f64)> = counts
                .iter()
                .filter(|&(_, &n)| n >= FINGERPRINT_FLOOR)
                .map(|(c, &n)| {
                    let rest = (totals[c] - n) as f64;
                    // +1 smoothing on the rest, so a kanji unique to this work
                    // scores high but not infinitely high.
                    let ratio = (n as f64 / here as f64) / ((rest + 1.0) / elsewhere as f64);
                    (c.to_string(), n, ratio)
                })
                .collect();
            ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then(b.1.cmp(&a.1)));
            ranked.truncate(FINGERPRINT_LEN);
            WorkFingerprint {
                work: work.clone(),
                encounters: here,
                unique: counts.len(),
                distinctive: ranked,
            }
        })
        .collect();
    out.sort_by(|a, b| b.encounters.cmp(&a.encounters));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: f64, text: &str, work: &str) -> KanjiLine {
        KanjiLine {
            ts,
            text: text.to_string(),
            work: (!work.is_empty()).then(|| work.to_string()),
        }
    }

    fn stats(lines: &[KanjiLine]) -> KanjiStats {
        aggregate_kanji(lines, &[], &[], &[], 4, 0)
    }

    #[test]
    fn counts_kanji_and_ignores_kana() {
        let s = stats(&[line(0.0, "「人が言った」", "A"), line(60.0, "人人", "A")]);
        let row = s.kanji.iter().find(|r| r.kanji == "人").unwrap();
        assert_eq!(row.count, 3);
        assert_eq!(s.total_encounters, 4);
        assert!(!s.kanji.iter().any(|r| r.kanji == "が"));
    }

    #[test]
    fn first_encounter_lands_on_one_day_only() {
        let day = 86400.0;
        let s = stats(&[
            line(day * 10.0 + 40000.0, "人", "A"),
            line(day * 11.0 + 40000.0, "人事", "A"),
        ]);
        assert_eq!(s.days.len(), 2);
        assert_eq!((s.days[0].new, s.days[0].cumulative), (1, 1));
        assert_eq!((s.days[1].new, s.days[1].cumulative), (1, 2));
        assert_eq!(s.days[1].encounters, 2);
    }

    #[test]
    fn grade_coverage_counts_met_and_solid() {
        let mut lines = vec![line(0.0, "人", "A"); SOLID_ENCOUNTERS as usize];
        lines.push(line(1.0, "事", "A"));
        let s = stats(&lines);
        let g1 = s.grades.iter().find(|g| g.grade == 1).unwrap();
        assert_eq!((g1.met, g1.solid), (1, 1)); // 人 is grade 1, met 10 times
        assert_eq!(g1.total, 80);
        let g3 = s.grades.iter().find(|g| g.grade == 3).unwrap();
        assert_eq!((g3.met, g3.solid), (1, 0)); // 事 is grade 3, met once
    }

    #[test]
    fn lookups_and_cards_join_by_containment() {
        let s = aggregate_kanji(
            &[line(0.0, "邂逅した", "A")],
            &[TermTimes {
                term: "邂逅".into(),
                times: 3,
            }],
            &["邂逅".to_string()],
            &[("邂逅".to_string(), 7), ("逅く".to_string(), 2)],
            4,
            0,
        );
        let row = s.kanji.iter().find(|r| r.kanji == "逅").unwrap();
        assert_eq!(row.lookups, 3);
        assert!(row.mined);
        assert_eq!(row.words, vec!["邂逅", "逅く"]);
        assert_eq!(row.lookup_rate(), 3.0);
    }

    #[test]
    fn bccwj_rank_rides_along() {
        let s = stats(&[line(0.0, "人邂", "A")]);
        let common = s.kanji.iter().find(|r| r.kanji == "人").unwrap();
        let rare = s.kanji.iter().find(|r| r.kanji == "邂").unwrap();
        assert_eq!(common.bccwj_rank, Some(1));
        assert!(rare.bccwj_rank.unwrap() > common.bccwj_rank.unwrap());
        assert!(common.bccwj_per_million.unwrap() > rare.bccwj_per_million.unwrap());
    }

    #[test]
    fn only_disproportionate_lookups_count_as_struggling() {
        // A corpus where 人 is read constantly and looked up once — the
        // baseline — against 邂, read at the floor and looked up every time.
        let mut lines = vec![line(0.0, "人", "A"); 100];
        lines.extend(vec![line(1.0, "邂", "A"); OUTLIER_ENCOUNTERS as usize]);
        // 逅 is looked up once on few sightings: a rate, but not an outlier's.
        lines.extend(vec![line(2.0, "逅", "A"); OUTLIER_ENCOUNTERS as usize]);
        let s = aggregate_kanji(
            &lines,
            &[
                TermTimes {
                    term: "人".into(),
                    times: 1,
                },
                TermTimes {
                    term: "邂".into(),
                    times: 6,
                },
                TermTimes {
                    term: "逅".into(),
                    times: 1,
                },
            ],
            &[],
            &[],
            4,
            0,
        );
        let row = |k: &str| s.kanji.iter().find(|r| r.kanji == k).unwrap();
        assert_eq!(s.baseline_lookup_rate, 8.0 / 112.0);
        assert!(row("邂").struggling);
        assert!(!row("逅").struggling);
        assert!(!row("人").struggling);
    }

    #[test]
    fn one_lookup_on_one_sighting_is_never_struggling() {
        let s = aggregate_kanji(
            &[line(0.0, "邂", "A")],
            &[TermTimes {
                term: "邂".into(),
                times: 1,
            }],
            &[],
            &[],
            4,
            0,
        );
        // Rate 1.0, the worst possible — but on a single encounter it is noise.
        assert_eq!(s.kanji[0].lookup_rate(), 1.0);
        assert!(!s.kanji[0].struggling);
    }

    #[test]
    fn fingerprint_favours_what_one_work_leans_on() {
        let mut lines = vec![line(0.0, "剣", "A"); FINGERPRINT_FLOOR as usize];
        lines.extend(vec![line(1.0, "人", "A"); 20]);
        lines.extend(vec![line(2.0, "人", "B"); 20]);
        let s = stats(&lines);
        let a = s.works.iter().find(|w| w.work == "A").unwrap();
        assert_eq!(a.distinctive[0].0, "剣");
        assert!(a.distinctive[0].2 > 1.0);
        let b = s.works.iter().find(|w| w.work == "B").unwrap();
        assert!(b.distinctive.iter().all(|d| d.0 != "剣"));
    }
}
