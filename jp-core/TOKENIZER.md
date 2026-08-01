# The tokenizer: how it works and where it breaks

Notes on `jp-core/src/tokenize.rs`, written while auditing its output. A record
of known defects, not a plan.

## How it works

Sudachi analyses the line. Each morpheme becomes a `Token` with four fields,
and **each field is decided by a different authority**:

| field | source |
| ------------- | ------------------------------------------------------- |
| `surface` | Sudachi, as written in the text |
| `base_form` | `written_form(normalized_form, dictionary_form)` — Sudachi's normalisation, overridden by Sankoku's spelling where the two disagree |
| `reading` | `dictionary_form_reading` — the reading of Sudachi's *lemma* entry, not of the surface |
| `pos` | Sudachi's top-level tag; `proper_noun` is the 固有名詞 subtag |

Around that sit three passes:

- **Dictionary-validated splitting** (C → B → A). Keep what Sankoku lists, split
  what it does not, progressively finer.
- **`decompose`** — a compound Sankoku does not list is cut into parts it does,
  longest match from the left. A one-character part must be kanji. Names are
  never decomposed.
- **`recompose`** — adjacent parts Sankoku lists as one word are rejoined. The
  kana half needs an unambiguous reading, so a reading naming more than one
  headword is dropped rather than guessed at.

`counts_as_word` then gates what reaches the ledger: a content word, or any
token whose `(headword, reading)` pair Sankoku lists.

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

## What breaks

### 1. The headword and the reading disagree

`written_form` picks a spelling against Sankoku's headword *set* and never looks
at the reading, while the reading comes from Sudachi's lemma entry. Nothing
reconciles them, so the pipeline invents pairs Sankoku does not list.

**334 of 15970 master rows** carry such a pair. Four shapes, one cause:

| shape | examples |
| ------------------------- | -------------------------------- |
| potential form as lemma | 行く/いける, 使う/つかえる, 言う/いえる |
| ざ行 variant | 信じる/しんずる, 感じる/かんずる |
| kana normalised to kanji | 一寸/ちょっと, 未だ/まだ, 唯/ただ, 毎/ごと |
| colloquial reading kept | 御前/おめえ, 無い/ねー, 全く/ったく |

### 2. Normalisation destroys a match that was there

綺麗 normalises to 奇麗. Sankoku lists 綺麗事 and does not list 奇麗事, so
`recompose` cannot rejoin 綺麗 + ごと into a word that is right there in the
dictionary. Yomitan finds 綺麗事 from the same text.

### 3. Multi-token expressions are not rejoined

それどころか is a Sankoku headword and arrives in pieces. なんだかんだ is not a
Sankoku headword, so leaving it in pieces is correct — the two look the same in
the output and are not the same problem.

### 4. Homographs the reading cannot separate

- **私** — Sudachi's lowest-cost entry for a bare 私 is わたくし, in every context
  tried except 私たち. Both 私/わたし and 私/わたくし are listed pairs, so no
  structural rule tells them apart. The ledger holds 804 encounters under
  わたくし against 122 under わたし.
- **うかがう** — 伺う and 窺う are distinct words sharing a reading. Only the
  surface separates them, and normalisation discards it.
- **味/み** — Sudachi normalises the nominalising suffix み of 哀しみ onto the
  homographic 味, and 味/み *is* a listed pair. Structurally valid, semantically
  wrong.

Separating these needs word frequency, and there is none loaded: the `BCCWJ`
dictionary row exists with **0 entries**, and `jp_core::text::bccwj_data` is a
character table, not a word table.

### 5. Mimetics and OOV kana shred

とんっと analyses as と + んっと. っ is a genuine Sankoku headword and so passes
the gate honestly, with 685 encounters. Sudachi has no entry for the mimetic and
the path cost produces fragments. Not fixable in this layer.

## Worth investigating: Yomitan/Nazeka-style deinflection

Yomitan resolves 綺麗ごと → 綺麗事 where we do not, and it does it without
Sudachi. Its approach is different in kind: instead of a statistical path
through a morpheme lattice, it takes the surface and applies deinflection rules
backwards, testing each candidate against the dictionary — longest match wins,
and the dictionary is the only arbiter.

That is worth a look because our defects cluster where the *statistical* choice
overrides the dictionary: a lemma we did not ask for (1), a normalisation that
loses a real headword (2), a cost model preferring わたくし (4). A dictionary-first
matcher has no opinion to override with. It would not help with 3 and 5, which
need better dictionary coverage rather than a better matcher, and it gives up
the part-of-speech tags and the lattice that make decomposition work at all.

The question is not whether to replace Sudachi, but whether a deinflecting
dictionary lookup should arbitrate where Sudachi's answer and Sankoku's entries
disagree.
