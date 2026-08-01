# Tokenizer identity rewrite: implementation plan

The plan for fixing the defects in `TOKENIZER.md`. One principle drives all of
it: **a ledger identity must be a `(headword, reading)` pair the master
dictionary lists, validated as a pair.** Today the headword and the reading are
chosen by different authorities (`written_form` picks a spelling against the
headword *set*; the reading comes independently from Sudachi's lemma) and
nothing ever checks them together — that is where every chimera comes from.

Sudachi stays. Segmentation, POS tags, the 固有名詞 name filter, and inflection
folding are not replaceable by a Yomitan-style deinflector, which has no concept
of names and matches substrings across word boundaries. What changes is the
layer that turns a morpheme into an identity, plus targeted extensions to
`recompose`. C→B→A splitting, `decompose`, and the name rules are untouched.

## Verified failure mechanics

Measured with `examples/probe.rs` (production setup: anki vocab as headwords,
master lexicon + readings loaded) against `system_full.dic`. Do not re-derive
these; they are the fixture set.

| input | today | mechanics |
| --- | --- | --- |
| とんっと | と + んっと (base=うんと, 感動詞, counts) | Sudachi has no とん mimetic path; んっと is a real entry normalising to うんと, a kana master headword, so `lists` passes on headword alone |
| それどころか | それ + どころ + か | Sudachi C splits it; それどころか **is** a Sankoku headword; `join_run` refuses because どころ and か are not content words |
| 敵の隙をうかがう | うかがう base=うかがう, counts as verb | token is fine; identity maps to no master pair — Sankoku has only 伺う/うかがう and 窺う/うかがう, so it never reaches the master scale |
| 食べてみる | みる base=見る | normalisation; Sankoku lists みる, いる, なる, おく, しまう, くる as their own kana headwords (the subsidiary senses). Same defect: 笑っていた → 居る |
| 行かなければならない | なら base=成る | same: subsidiary なる written as kana must stay なる |
| 私なら行けるはずだ | 行ける base=行く read=イケル | Sudachi normalises the potential to the base verb but keeps the potential's reading → chimera (行く, いける). Master lists 行く/いく and 行く/ゆく |
| 私なら行けるはずだ | はず base=筈 | master lists only kana はず — (筈, はず) is a chimera |
| ちょっと待って | base=一寸 read=チョット | master lists 一寸/いっすん only → chimera (一寸, ちょっと); the kana pair ちょっと/ちょっと is listed |
| 綺麗ごとを言うな | 綺麗(base=奇麗) + ごと(base=毎, 接尾辞) | both 綺麗 and 奇麗 are master headwords so normalisation stands; join fails twice — ごと is 接尾辞 (content-word fence) and the reading join is fenced to verb+verb |
| 私 | read=ワタクシ | (私, わたくし) is a *listed* pair, so pair validation cannot fix it — needs reading-aware frequency (stage 7, deferred) |

BCCWJ **word** frequency exists: `dictionary_frequency` has 886k rows
(私=47, 伺う=1886, 窺う=2831, とん=12882). `TOKENIZER.md`'s "0 entries" claim
looked at `dictionary_entries`. The table has no reading column — the Yomitan
parser (`dictionary/mod.rs`, `parse_frequency_banks`) sees the reading and
drops it.

## Stage 0 — fixtures and red tests

1. Keep `examples/probe.rs` (uncommitted so far; commit it). Add a `--corpus
   FILE` mode: read lines from a text file, print only tokens whose identity is
   not a master pair, for before/after diffing over real lines
   (`sqlite3 knowledge.db "select text from lines" > lines.txt`).
2. New integration test `tests/identity_resolution.rs`, `#[ignore]`d like the
   existing compound test, run with
   `JP_TOOLS_SUDACHI_DICT_PATH=$PWD/../system_full.dic cargo test -p jp-core --test identity_resolution -- --ignored`.
3. Check in `tests/master_pairs.tsv` — a hand-picked subset of Sankoku pairs so
   the test does not depend on the live DB. Regenerate rows from:
   `sqlite3 -separator $'\t' ~/.local/share/jp-tools/knowledge.db "select distinct term, reading from dictionary_entries where dictionary_id=1"`.
   Needed rows at minimum: それどころか, みる, いる, なる, おく, しまう, くる,
   見る/みる, 居る/いる, 成る/なる, 伺う/うかがう, 窺う/うかがう, 行く/いく,
   行く/ゆく, はず, ちょっと, 綺麗/きれい, 奇麗/きれい, 綺麗事/きれいごと,
   毎/ごと, うんと, する, 言う/いう, 食べる/たべる, 敵/てき, 隙/すき,
   押す/おす, 軽い/かるい, 胸/むね, 待つ/まつ, 私/わたし, 私/わたくし,
   申し訳ない/もうしわけない, 振り返る/ふりかえる (the last two are recompose
   regression guards).
4. Write the test matrix below as assertions. All new-behavior cases fail red;
   the regression cases pass and must stay green through every stage.

## Stage 1 — `Token.subsidiary` and the kana-identity rule

`Token` gains `pub subsidiary: bool` — true when `part_of_speech()[1]` is
`非自立可能` (same shape as the existing `proper_noun` flag; `to_token` in
`tokenize()` sets it).

Rule, applied before everything else in identity resolution: **subsidiary +
all-kana surface → identity is the kana lemma**, i.e. `(dictionary_form,
dictionary_form_reading)`. Sudachi's `dictionary_form` preserves the surface's
orthography (surface い → いる, not 居る), so this is exactly the subsidiary
headword Sankoku itself lists. Fixes 食べてみる→みる, ていた→いる,
なければならない→なる. A subsidiary written in kanji (見てみる's 見る head) is
untouched.

This deliberately still counts subsidiaries as encounters — under the right
headword, which Sankoku separates from the kanji verb for us. If they should be
suppressed entirely later, the flag is the hook; do not decide that here.

## Stage 2 — pair-validated identity resolution (the core)

Replace the `written_form(...)` + `dictionary_form_reading(...)` pair in
`to_token` with one function, `resolve_identity(&self, m) -> (String, String)`.
The tokenizer needs the master *pairs*, not just the headword set:
`with_master_readings` already receives the full entries — additionally build a
`MasterWords`-equivalent pair set there (or store a `MasterWords` in the
tokenizer; dedupe the type rather than duplicating it — ingest builds one
anyway, `Arc` it in).

Candidate ladder; first candidate that validates (via `MasterWords::lists`,
which handles the kana-headword-without-reading case and hiragana folding)
wins:

1. **subsidiary-kana** (stage 1's rule).
2. **(normalized_form, sudachi reading)** — today's behavior. Trying it first
   preserves orthography folding: いう/言う keep collapsing to one row.
3. **(dictionary_form, sudachi reading)** — surface-faithful spelling. Catches
   する (為る→する), ちょっと (一寸→ちょっと), and keeps kana spellings the
   master lists as kana.
4. **(surface, surface reading)** — for uninflected tokens where even the
   lemma spelling misses (はず when `dictionary_form` disagrees; verify with
   the probe which field actually carries 筈).
5. **re-derived reading**: for each spelling S in {normalized, dictionary,
   surface} that *is* a master headword but failed on reading, tokenize S
   standalone (Mode C, expect one morpheme) and validate (S, its reading).
   This repairs the potential-form chimera: (行く, いける) fails, re-tokenised
   行く reads イク, (行く, いく) validates. Cache results in a
   `Mutex<HashMap<String, String>>` on the tokenizer — it fires rarely but the
   same words recur forever.
6. **reading fallback**: fold the reading (or the surface, if all kana) to
   hiragana, look up the master headwords under it. Unique → take it, with the
   master's reading. Ambiguous → stage 5's frequency pick. This is what maps
   うかがう → 伺う/窺う at all.
7. **nothing validates** → keep candidate 2's values unchanged (today's
   output). No regression: the token stays off the master scale exactly as it
   does now.

Notes:
- All reading comparisons in hiragana (`text::kana::to_hiragana`); Sudachi
  returns katakana.
- Steps 2–4 are three lookups; the ladder costs nothing on the common path
  (step 2 hits).
- Delete `written_form` once nothing calls it.
- `dictionary_form_reading` stays — it is the "sudachi reading" of steps 2–3.

## Stage 3 — impossible-onset guard

No Japanese word starts with っ, ん, or a small kana (ゃゅょぁぃぅぇぉ, and
katakana equivalents). A token whose **surface** starts with one is a shred
from an OOV path. Rule, in `resolve_identity` before the ladder: such a token
skips candidates 2–6 (no rescue through normalisation — んっと must not become
うんと) and, in `counts_as_word`, never counts. Simplest form: give the guard
its own predicate in `tokenize.rs` and call it from both places.

Consequence, deliberate: Sankoku's っ headword becomes uncountable. Its 685
ledger encounters are all shreds; that is the point.

とん itself stays unfixed — mimetic coverage is a Sudachi user-dictionary
project (out of scope; noted in TOKENIZER.md §5). The guard just stops the
shreds from being ledgered as words.

## Stage 4 — recompose extensions

Three changes to `recompose`/`join_run`, keeping `MAX_COMPOUND_PARTS = 3` (それ
+ どころ + か fits; raise only with evidence):

1. **Expression join.** New spelling signal: concatenated **surfaces** equal a
   master headword → join, even when parts are function words (どころ, か).
   Constraints: no proper noun in the run, joined length ≥ 3 chars, exact
   headword match. The existing all-content-words fence applies only to the old
   base_form-concat signal, not this one. Function-word joins are safe *because*
   the joined string must be a listed headword — the master dictionary decides
   wordhood. Fixes それどころか.
2. **Reading join, kanji-head fence.** Today's reading signal is fenced to
   verb+verb with kana heads. Add a second admission: the head parts contain at
   least one kanji character (綺麗 + ごと → reading きれいごと → 綺麗事). The
   disasters that motivated the fence (そう+する→相する, こと+し→今年) are
   all-kana heads and stay fenced out. The 接尾辞 part must be admitted for
   this path — relax the content-word check to also accept 接尾辞 when the
   kanji-head condition holds. Build the reading from **surface readings**
   (`m.reading_form()` equivalents are already on the token as… they are not:
   `Token.reading` is the lemma reading. Use `to_hiragana(surface)` for kana
   parts as `join_run` already does, and the token's reading for kanji parts —
   for 綺麗 the lemma reading キレイ is the surface reading, which holds for
   uninflected heads generally; the last part keeps using `last.reading`.)
3. **Joined-token identity** goes through stage 2's ladder too (the joined
   token currently takes `term_reading`'s first reading — fine, but validate
   the pair; if the join produces a non-listed pair something is wrong, drop
   the join).

## Stage 5 — frequency arbitration for ambiguous readings

1. `with_master_readings`: keep the ambiguous readings instead of dropping
   them — `by_reading: HashMap<String, Vec<String>>`. The *join* path
   (recompose) keeps requiring a unique reading: merging two tokens on a
   frequency guess is still worse than leaving them apart. Only identity
   fallback (stage 2, step 6) may arbitrate.
2. New builder `with_frequency(ranks: HashMap<String, i64>)`. Picker: among the
   headwords under a reading, take the lowest rank; no rank for any candidate →
   refuse (behave as today's unique-only rule). うかがう → 伺う (1886) over
   窺う (2831). Sometimes wrong — 隙をうかがう is 窺う — and there is no
   context-free right answer; wrong-but-adjacent beats never-counted. Accepted.
3. Plumbing: `knowledge::dictionaries` gets
   `frequency_ranks(pool, dict_title_or_id, terms: &[String]) -> HashMap<String, i64>`
   (batched `IN` queries, `idx_dictionary_frequency_lookup` covers it). In
   `read-stats/src/ingest.rs`, compute the ambiguous-headword set (expose it
   from the tokenizer or recompute from `master_readings`), fetch ranks for
   just those terms **before** `spawn_blocking`, pass the map in. Both call
   sites in ingest.rs (~line 278 and ~356) get the same treatment.

## Stage 6 — eval, docs, rollout

1. Full test suite green, including the two `#[ignore]` integration tests.
2. Corpus diff: run `probe --corpus` over all `lines` text, before vs after.
   Review: (a) count of tokens with non-master identities should drop
   substantially; (b) eyeball a random 50-line diff for regressions the matrix
   missed.
3. Re-run the §1 audit: ledger rows whose pair Sankoku does not list — the
   334 should collapse to near zero **for newly ingested lines**.
4. Update `TOKENIZER.md`: fix the stale BCCWJ claim, move fixed defects into
   "what works", leave §5 (mimetics) and 私 (until stage 7) as open.
5. Restart read-stats via `scripts/start-all.sh` — not during a VN session; use
   `scripts/dev-instance.sh` to verify endpoints first.

**Open decision for the user (ask, do not assume): re-harvest history?** The
`lines` table holds every line ever read, so encounters (`word_days`,
`work_terms`) can be rebuilt under the new identities by resetting the
watermarks. `vocabulary.status` is human judgment keyed on the old identities —
rows like (見る,みる) stay valid, but 804 encounters sit under (私,わたくし)
and statuses may exist on identities that stop being produced (居る/いる vs
new いる). Rebuilding encounters is safe and derived; migrating statuses needs
a mapping table of moved identities. Scope it only when asked.

## Stage 7 — done, but not as planned: the 私 problem

The plan was to add a `reading` column to `dictionary_frequency`, re-import
BCCWJ, and prefer the (headword, other-reading) that is decisively more
frequent. **The premise was wrong.** BCCWJ is annotated with the same UniDic
Sudachi uses, and ranks 私/わたくし at 47 against 私/わたし at 182 — asking it
would have confirmed the error rather than fixed it.

What works is Jitendex's `score`, which is JMdict's priority tagging: editorial,
independent of UniDic, and it scores わたし 200 against わたくし 0. So:

- The reading column and the parser change happened anyway (`FreqEntry`), but
  BCCWJ's job is only to break ties between readings that are *equally* current
  — 私 is also あたし at 200, and the corpus says わたし is the commoner of the
  two.
- `dictionaries::preferred_readings` derives, per headword, a preferred reading
  and the set of readings not to touch. `POPULARITY_TIER = 150` was chosen
  against the corpus, not guessed: one tier (100) also rewrote 街/まち and
  身体/からだ, which score 99 and are plainly real readings.
- A negative score is JMdict tagging the *spelling* (居る/いる is -101 because
  いる is usually written in kana), so a reading scored negative is never
  corrected away — believing it inverted 居る to おる.
- The correction applies only when the **surface is bare kanji**. Kana in the
  text is the text's own answer, not Sudachi's guess.

Measured over the `lines` corpus: 1,091 tokens move, 796 of them 私 → わたし, the
rest bare kanji Sudachi gave on-readings (所/ショ → ところ, 者/シャ → もの,
薬/ヤク → くすり). Known cost: 人気/ひとけ becomes にんき.

## Test matrix

Every row is an assertion in `tests/identity_resolution.rs`. "identity" means
`(base_form, reading)` after hiragana folding; "counts" means
`counts_as_word`.

| input | assert |
| --- | --- |
| そのメルルの胸を、とんっと軽く押した。 | no token with base うんと; no counting token whose surface starts with っ/ん; 胸, 軽い, 押す present |
| それどころか、彼は笑っていた。 | one token それどころか/それどころか; ている-token identity いる not 居る |
| 敵の隙をうかがう | verb identity 伺う/うかがう (frequency pick); counts |
| 食べてみる | みる/みる, not 見る |
| 行かなければならない | なら-token identity なる/なる, not 成る |
| 私なら行けるはずだ | 行ける-token identity 行く/いく; はず/はず not 筈 |
| ちょっと待って | ちょっと/ちょっと not 一寸 |
| 綺麗ごとを言うな | joined 綺麗事/きれいごと |
| 見てみる | first 見る stays 見る/みる (kanji subsidiary untouched) |
| する形 (any する sentence) | する/する (regression) |
| 知る inflections (しっ/しら/しる) | all fold to 知る/しる (regression) |
| 申し訳ない, 振り返って | recompose still joins (regression) |
| 東京 (proper noun context) | not decomposed (regression) |

## Gotchas

- Readings are katakana out of Sudachi, hiragana in the master maps: fold at
  every comparison, once, in one place.
- `term_reading` is first-insert-wins; with multi-reading headwords (私 has
  six) never use it as "the" reading for a headword — only step 6's explicit
  pick or the validated pair itself decide.
- Data for `spawn_blocking` must be prefetched async (ingest.rs pattern);
  frequency ranks included.
- The Sudachi dict lives at the repo root (`system_full.dic`); tests need
  `JP_TOOLS_SUDACHI_DICT_PATH` and `--ignored`.
- rust-analyzer may flag `mockall` in `tokenize.rs` when checking without the
  `test-support` feature; it is noise.
- Never restart the live stack during a VN session; `dev-instance.sh` exists
  for exactly this work.
