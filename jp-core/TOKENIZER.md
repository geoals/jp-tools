# The tokenizer: how it works and where it breaks

Notes on `jp-core/src/tokenize.rs`, written while auditing its output. A record
of known defects, not a plan.

## How it works

Sudachi analyses the line. Each morpheme becomes a `Token`, and the identity the
ledger keys on — `(base_form, reading)` — is chosen **as a pair**, by
`resolve_identity`:

| field | source |
| ------------- | ------------------------------------------------------- |
| `surface` | Sudachi, as written in the text |
| `base_form`, `reading` | `resolve_identity` — candidate spellings and readings tried in order, first pair Sankoku lists wins |
| `pos` | Sudachi's top-level tag; `proper_noun` is the 固有名詞 subtag, `subsidiary` the 非自立可能 one |

The ladder, in order: the kana lemma of a subsidiary; Sudachi's normalisation;
its dictionary form; the surface; a re-derived reading (the spelling tokenized
alone, which repairs a potential form's reading); and last, for a hiragana
surface only, the headword its reading names. The winning pair's *reading* is
then corrected where Sudachi is known to guess a dead one — see
`dictionaries::preferred_readings`, which is the only thing here allowed to
overrule a listed pair.

Around that sit three passes:

- **Dictionary-validated splitting** (C → B → A). Keep what Sankoku lists, split
  what it does not, progressively finer.
- **`decompose`** — a compound Sankoku does not list is cut into parts it does,
  longest match from the left. A one-character part must be kanji. Names are
  never decomposed.
- **`recompose`** — adjacent parts Sankoku lists as one word are rejoined, on the
  spelling, on the whole run's surfaces, or on the reading. The reading is the
  weak signal and stays fenced to verb runs and kanji-headed compounds; joining
  never arbitrates an ambiguous reading.

`counts_as_word` then gates what reaches the ledger: a content word, or any
token whose `(headword, reading)` pair Sankoku lists — and never a token with an
impossible onset.

## What works

- Inflection folds correctly. 知る no longer splits across しる, しら, しっ —
  the lemma reading is what makes one word one row.
- Compounds Sudachi keeps whole are recovered: 懲罰房 → 懲罰 + 房 when it is not
  tagged a name.
- Names stay out. The 固有名詞 verdict is per term over a whole pass, so a cast
  list does not become vocabulary.
- Orthography follows Sankoku where the two dictionaries disagree, so する does
  not become 為る and fall out of the queue.
- The wordhood gate is a single pair test with no stoplist behind it.
- **The identity is a pair Sankoku lists**, or it is Sudachi's own answer left
  alone. 行く/いける, 一寸/ちょっと and 未だ/まだ are gone; over the whole `lines`
  corpus the identities Sankoku does not list fell from 1,023 distinct to 610,
  and the survivors are names and mimetics.
- **Subsidiary verbs go to their own headword.** Sankoku lists the て-form
  auxiliaries as kana headwords, so 食べてみる credits みる and not 見る, 笑って
  いた credits いる and not 居る.
- **Expressions rejoin across function words** when the joined surface is itself
  a headword: それどころか, じゃない, として.
- **A guessed reading can be overruled by the dictionaries.** Both 私/わたし and
  私/わたくし are listed pairs and Sudachi read every bare 私 as わたくし; the
  ledger had 893 encounters on a reading almost nothing in a visual novel is. It
  now reads わたし. Only where the surface is bare kanji — when the text spells
  わたくし out in kana, that is not a guess and it stands.
- **Shreds do not count.** No word begins with っ, ん or a small kana, so a token
  that does is refused — including the 750 sightings of っ and every っ-initial
  fragment off an OOV mimetic path.

## What breaks

### 1. Homographs the reading cannot separate

- **うかがう** — 伺う and 窺う are distinct words sharing a reading, and nothing
  in the sentence says which. It is resolved by BCCWJ rank, so 隙をうかがう is
  credited to 伺う and is sometimes wrong. Deliberate: wrong-but-adjacent beats
  never counted.
- **味/み** — Sudachi normalises the nominalising suffix み of 哀しみ onto the
  homographic 味, and 味/み *is* a listed pair. Structurally valid, semantically
  wrong.
- **人気/ひとけ** — the reading preference above rewrites it to にんき, which is
  wrong in 人気のない道. It is the one case where the correction is known to cost
  something, against ~1,100 tokens it fixes.
- **何** — bare 何 comes out ナン every time: 1,000 occurrences over 17k lines,
  not one of them ナニ, where roughly three quarters (何を, 何が, 何も, 何？)
  should be. Not fixable here, and not the general defect it looks like —
  Sudachi resolves context-dependent readings correctly for most words (時 is
  トキ 315 and ジ 57, 様 is サマ 215 and ヨウ 110). It simply never selects a
  bare 何/ナニ path, so there is no analysis to prefer, only a rule one could
  hand-write — which is what §3 is for.

### 2. Mimetics and OOV kana shred

とんっと analyses as と + んっと. Sudachi has no entry for the mimetic and the
path cost produces fragments. The fragments no longer count as words, but the
mimetic is still not read as one — that needs a Sudachi user dictionary, not a
change in this layer.

### 3. Names are most of what is left

The identities Sankoku does not list are now dominated by cast lists and place
names (エマ, アリサ, 羽咲) that the 固有名詞 majority verdict did not catch, plus
text that is not Japanese at all — engine markup and stray latin letters in the
`lines` table. Neither is a tokenizer defect.

### 4. Where a correction to the analyzer belongs

Two defects here — the mimetic gap and 何 — are Sudachi producing an analysis no
logic downstream can repair, because the reading or the segmentation it needs is
one it never emits. The mechanism for that is a **Sudachi user dictionary**: data
in the analyzer, where the cost model still lets context decide, rather than
Japanese grammar hand-written into `tokenize.rs` one word at a time. Nothing here
does that yet; it needs a user-dic CSV, a compile step and a rebuild hook.

## Worth investigating: Yomitan/Nazeka-style deinflection

Yomitan resolved 綺麗ごと → 綺麗事 before we did, and it does it without Sudachi.
Its approach is different in kind: instead of a statistical path through a
morpheme lattice, it takes the surface and applies deinflection rules backwards,
testing each candidate against the dictionary — longest match wins, and the
dictionary is the only arbiter.

The defects it would have addressed were the ones where the *statistical* choice
overrode the dictionary, and the identity ladder addresses those by letting the
dictionary refuse Sudachi's answer instead. It gives up the part-of-speech tags
and the lattice that make decomposition and the name filter work at all, and it
would not have helped with 私 — a deinflector has no opinion about which of two
listed readings a bare kanji has either.
