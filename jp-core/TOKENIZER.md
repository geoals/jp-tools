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

Around that sit two passes:

- **Dictionary-validated splitting** (C → B → A). Keep what Sankoku lists, split
  what it does not, progressively finer.
- **`recompose`** — adjacent parts Sankoku lists as one word are rejoined, on the
  spelling, on the whole run's surfaces, or on the reading. The reading is the
  weak signal and stays fenced to verb runs and kanji-headed compounds; joining
  never arbitrates an ambiguous reading.

Two rules keep a *form* from being mistaken for a word, which is what every
matching rule here keeps rediscovering — Japanese lists a word for a great many
short kana strings, so 続い + て spells the conjunction 続いて and 許せ is an entry
of its own:

- **structural** — a surface that is not its own dictionary form is a stem, and
  a stem is neither an identity nor a part of a surface join.
- **the dictionary's word class** — Yomitan term-bank field 3, which Sankoku
  fills in: 許す is `v5`, 許せ is nothing. An inflected token's identity has to be
  an entry that conjugates. Kana entries are exempt, since the auxiliaries (みる,
  いる, なる) carry no tag either.

The second is currently redundant — it changes no identity over the whole read
corpus that the first does not already fix — and is kept as the one that states
the actual reason, so a future rule cannot quietly rediscover 許せ.

`counts_as_word` then gates what reaches the ledger: a content word, or any
token whose `(headword, reading)` pair Sankoku lists — and never a token with an
impossible onset.

## The regression net

`tests/golden.rs` runs 250 real sentences through the tokenizer and snapshots the
identities the ledger would get, fixtures beside it so it never reads a live
database. It is the only thing here that shows a rule's whole blast radius rather
than the cases someone thought of, and it has already caught a change that looked
right and flooded the output with glued expressions (じゃない, として, ように).

Read its diff; do not regenerate past it.

## What works

- Inflection folds correctly. 知る no longer splits across しる, しら, しっ —
  the lemma reading is what makes one word one row.
- Compounds Sudachi hands over in pieces are put back: 一件, 一年, 神様, 人達,
  室内 — 137 sightings the two-kanji join recovers. A compound Sankoku does not
  list, like 懲罰房, is left whole rather than cut into parts it does list;
  `decompose` did the cutting and destroyed more words than it credited.
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
- **A bound kanji keeps the reading its binding gives it.** The reading
  correction is what the popularity dictionary scored, and it scored the
  free-standing word: 名 is な, but 数名 is メイ. Sudachi tags the two apart
  (接頭辞, 接尾辞, 助数詞可能), so the fence is its own tag rather than a list.
  It was overruling 32 headwords — 名/メイ 21, 者/シャ 39, 薬/ヤク 35, 所/ショ
  46, 日/カ 9 — about 230 tokens, every one of them wrong.
- **One mora of kana never becomes a kanji word.** Japanese has a kanji for
  every mora, so the normalisation is always available and never evidence.
  UniDic sent the か of 何もかも to the archaic pronoun 彼, the nominalising み of
  哀しみ to 味, the honorific お to 御 (196), the plural ら to 等 (156), 「く」 to
  九 (102) and 「ちょ、マジで」 to 一寸 (20) — all listed pairs, none of them the
  word anyone read. The reading fallback already refused this; the normalisation
  path did not.

## What breaks

### 1. Homographs the reading cannot separate

- **うかがう** — 伺う and 窺う are distinct words sharing a reading, and nothing
  in the sentence says which. It is resolved by BCCWJ rank, so 隙をうかがう is
  credited to 伺う and is sometimes wrong. Deliberate: wrong-but-adjacent beats
  never counted.
- **人気/ひとけ** — the reading preference above rewrites it to にんき, which is
  wrong in 人気のない道. Its remaining cost, now that bound morphemes are fenced
  out of the correction: 相/しょう (4), 陽/よう (2), against ~1,300 it fixes.
  Both are single kanji Sudachi tags 名詞,普通名詞,一般 — the same tag the free
  noun carries — so nothing in the analysis separates them.
- **今/こん** (5) — the reverse: Sudachi mis-tags the 今 of 「つまり今アンアン
  ちゃんが」 a 接頭辞, and the preference rule used to mask that by rewriting it
  to いま. The bound-morpheme fence now believes the tag, so one error is no
  longer hidden behind another. Faithful, and wrong in the same 5 places.
- **何** — bare 何 comes out ナン every time: 1,000 occurrences over 17k lines,
  not one of them ナニ, where roughly three quarters (何を, 何が, 何も, 何？)
  should be. Not fixable here, and not the general defect it looks like —
  Sudachi resolves context-dependent readings correctly for most words (時 is
  トキ 315 and ジ 57, 様 is サマ 215 and ヨウ 110). It simply never selects a
  bare 何/ナニ path, so there is no analysis to prefer, only a rule one could
  hand-write — which is what §6 is for.

### 2. Words the join is fenced away from, and one it cannot reach

Each was measured and left, because the fence that blocks it is worth more than
the word it costs.

- **もしかして** is a Sankoku headword, and Sudachi gives もしか + し + て. The
  surface join refuses because し is a stem, which is the rule that stops
  続い + て spelling the conjunction 続いて and そう + な the hearsay そうな.
- **きれいごと** does not join, though 綺麗ごと does. The reading join needs a
  kanji head, and all-kana heads are what invent こと + し → 今年 and
  時 + 前 → 自前.
- **おじさん** (57) cannot join at all. Sankoku has 伯父さん and 小父さん under
  おじさん and no kana headword, so the surface never spells an entry and the
  reading names two — and a join, unlike an identity, is never arbitrated by
  frequency. Merging two tokens on a guess is worse than leaving them apart.
- **はがいじめ** is Sudachi segmenting は + が + いじめ. 羽交い締め parses fine;
  the kana spelling is a lexicon gap, so it belongs in §5.

### 3. Spellings Sankoku has under another headword

舌舐り (Sankoku: 舌舐めずり), 見真似 (見様見真似), 途轍 (途轍もない) — the reading
is right and the lemma spelling is Sudachi's, not the master's. The reading
fallback would find them, but it is fenced to hiragana lemmas because a reading
is the weakest signal there is: katakana turned エマ into 絵馬 and a stray latin
`g` into グラム 14,314 times. Extending it to kanji lemmas needs a narrower
admission than "unique reading" — the first character shared, at least — and it
was not measured. 羽ばたき, 凛と, 触り心地, 先走り and 覗かす are not this: Sankoku
lists 羽ばたく, 凛, 先走る and no noun, so those are correctly not vocabulary.

### 4. Mimetics and OOV kana shred

とんっと analyses as と + んっと. Sudachi has no entry for the mimetic and the
path cost produces fragments. The fragments no longer count as words, but the
mimetic is still not read as one — that needs a Sudachi user dictionary, not a
change in this layer.

### 5. Names are most of what is left

The identities Sankoku does not list are now dominated by cast lists and place
names (エマ, アリサ, 羽咲) that the 固有名詞 majority verdict did not catch, plus
text that is not Japanese at all — engine markup and stray latin letters in the
`lines` table. Neither is a tokenizer defect.

### 6. Where a correction to the analyzer belongs

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
