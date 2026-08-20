# read-stats — daily reading tracker + the `#read` reading view

Rust 2024. Axum JSON API + Preact/htm frontend (no build step), two SQLite
databases. Port 3200.

- **the dashboard** — how much was read, how fast, how continuously, what it
  cost in lookups. Everything is derived from the raw line stream at query time,
  so changing a threshold re-reads the whole history under the new rule.
- **`#read`** — the live line feed read beside the running VN, the explain
  button, and the AnkiConnect proxy Yomitan points at.

## The shape of the thing

```
                  vn-ws-logger.py                     Yomitan
                        │ appends                        │ AnkiConnect
                        ▼                                ▼
                 knowledge.db: lines            routes/ankiproxy.rs
                        │                                │ records
                        ▼                                ▼
   history.rs  ◄─── one load per request ───►   knowledge.db: lookups
        │
        ▼
    stats/ ── pure derivation ──►  routes/ ──► JSON ──► static/
```

Nothing in `stats/` touches a database, a clock or a timezone; every threshold
arrives as a parameter. That is what lets `tests/api.rs` assert exact numbers.

`src/lib.rs` is the layer map. Read `stats/presence.rs` first — how much of a
gap counts as reading is the decision everything else builds on. `clock.rs`
holds the only impure inputs; each `db/` module doc says which database it
talks to.

## Two databases

`knowledge.db` is shared and its schema is owned by `jp_core::knowledge`
(`lines`, `works`, `manual_sessions`, `anki_notes`, `word_days`, `lookups`,
`vocabulary`, the dictionary cache). `read-stats.db` is this app's own:
`settings`, `reader_marks`, `work_covers`. `db` functions take a `Knowledge`
handle or a bare `SqlitePool`, so passing the wrong database is a compile error.
The two places that straddle the line — the current work's capture window and
the cover sources — join in memory; keep it that way.

## Invariants

Measurement:

- **Presence is the rule everything credits time through.** A new aggregate
  that measures time goes through `stats::Presence`, not a fresh
  `min(gap, cap)`. When those diverged, the focus metric punished the reader for
  using a dictionary.
- **Pace is a property of the reader, not of a request.** `History` derives it
  once over all history, or the dashboard and the day timeline disagree about
  the same day.
- **Speed divides by measured reading only** (`History::measured_days`). An
  untimed session's duration is derived from the reader's own pace, so in a
  speed chart it would measure its own output. Totals, goals and streaks still
  count everything read.
- **Exposure counts take all text; cost counts take only hooked text.** Pasted
  session `content` feeds `word_days`, the kanji grid and every coverage figure,
  but stays out of every rate — `lookups_per_1k` divides by hooked characters.
- **A book read on paper is logged against its epub, and the epub is the
  count.** `books` holds one flattening of the file and every position is a byte
  offset into it, so the text is stored rather than the path — a re-parse under
  a changed stripper would move every offset already recorded. A sitting is
  named by where it *ended*: an anchor typed off the page, searched **forward
  only** from the last position, which is what makes ten characters safe when
  the same ten occur earlier in the book. What lands in `manual_sessions` is an
  ordinary row carrying the span between the two positions, so nothing
  downstream knows it came from paper.
- **Catching up a part-read book moves the position without writing a
  session** (`/api/books/skip`). Those pages were read before there was
  anything to record them with; logging them would credit a day that never
  happened and push the whole span through the ledger as freshly met.
- **Chars per page comes from the pages the body runs between**, not from the
  book's total page count — a total counts the blanks, the TOC and the
  afterword, so every page estimate would read high.
- **`chars` excludes punctuation** (`jp_core::text::chars`), matched to
  texthooker-ui so speeds are comparable with other people's. Startup recounts
  the column.

The line stream:

- **Nothing is deleted.** A line that shouldn't count gets `discarded = 1`,
  filtered on read.
- **Pausing stops capture, it does not filter.** vn-ws-logger.py polls
  `settings.capture_paused` and closes its Textractor WebSocket while it is set,
  so a paused span simply has no lines in it.
- **A lookup only exists if it happened while reading.** Yomitan fires the proxy
  for anything looked up anywhere, so `ankiproxy::record` records only when a
  line arrived within `session_gap_secs`. The guard is at the write and nowhere
  else — don't add a second filter downstream.

The ledger (`vocabulary`):

- **Only the reader writes `status`.** Not ingest, not the Anki sync, not the
  lookup sync — a resync must never demote a word marked known, and an encounter
  count must never promote one. Today's writers:
  `/api/vocab/judge`, `/api/vocab/blacklist-non-words`, the tap in `#read`, and
  the `anki-import` / `jiten-import` / frequency imports.
- **`new` ≠ `unknown`.** `new` means never judged; collapsing them is
  irreversible and breaks the triage progress figure.
- **Anki owns mined-state.** `anki_notes` is a snapshot, replaced wholesale,
  never written back. **`card::add_note` adds the one card Anki just accepted**
  (`db::insert_anki_note`), which does not make the table a source of truth —
  the refresh still replaces it — but without it the mirror only learns about
  cards when the dashboard page opens. Every cards-per-hour figure counts rows
  here, so an evening of mining in the overlay read as zero. It is its own
  spawned task rather than a step in `enrich_added_note`, because it resolves a
  ledger key through Sudachi and nothing may be awaited in front of the
  capture. `vocabulary.mined` is recomputed from it and is a flag
  beside `status`, never written into it.
- **A word judged under one reading is not asked about again**, and not marked
  under another either. 皆 marked known as みな means 皆/みんな is never offered.
- **A card and a lookup are spelt as they were written; the ledger keys on the
  normalized form.** `anki_notes.headword` and `lookups.headword` hold the
  resolved key and are what joins to `vocabulary.headword` — never the raw
  `vocab`/`term`, which is how 検死 and 検屍 became two rows that each looked
  empty. See the root CLAUDE.md for the full account. Both are filled by
  `ingest::normalized_spellings`: `anki_notes` on the refresh that replaces the
  snapshot, `lookups` by `ingest::normalize_new_lookups` just before
  `sync_lookup_counts` reads it — not at write time, because `ankiproxy::record`
  is on the mining hot path and would pay a Sudachi load per popup.
- **Each ingest sink has its own watermark.** One pass fills `word_days`, the
  ledger and `work_terms`, but their three watermarks move independently. The
  sinks are additive and not idempotent, so a row goes to a sink only when its
  id is past _that sink's_ mark — which is what lets `POST /api/vocab/rebuild`
  re-derive the ledger without double-counting.

Tokenization (all in `jp_core::tokenize`, shared with the highlighter so a tint
and a ledger row cannot disagree):

- **The line is rewritten once before Sudachi sees it, and only once.** The
  emphatic っ — a small っ with only punctuation or the end of the line after it
  — is stripped (`strip_emphatic_sokuon`). It has to happen there: an analysis
  ending in a 促音便 can absorb that っ where the real word cannot, so the
  lattice prefers it and はいっ comes back as 入る, まずいですっ as まず + 出る +
  素っ. No rule over the finished tokens can undo that. Nothing else edits the
  input, and nothing should — every character removed is a っ, which is what
  keeps every surface findable in the original line for `locate`'s offsets.
- **A term's reading is the reading of its headword**, not of the surface —
  otherwise 知る splits across しる, しら and しっ.
- **One word, one row, spelt the way the master dictionary spells it.** Terms
  key on Sudachi's _normalized_ form. Where Sudachi and Sankoku disagree,
  Sankoku wins (`written_form`).
  **Except where the reader's own spelling is a Sankoku headword too** — then it
  wins, because there is nothing left to decide: 綺麗 and 奇麗 are both listed,
  the page said one of them, and normalising 検死 to 検屍 or 上手く to 旨い
  asserts a word nobody read. 406 tokens over 116 spellings. The rule only
  reaches a surface that is a headword as written, so it never costs the scale;
  an inflected stem (舐め, 穢さ) is not a word and is still normalised.
- **The kana alphabet is part of the spelling.** Sudachi folds ザル onto ざる
  and マジ onto まじ, and Sankoku lists each as two words — the colander and the
  slang against the classical negative. A katakana surface Sankoku lists and
  reads in hiragana keeps its own spelling; a katakana entry read in katakana is
  a loanword (モノ is monochrome), so モノ still folds onto もの, and サクラ →
  桜 still folds because that is orthography and not the alphabet. The other
  direction is a word too: where the master lists **only** the hiragana, the
  katakana spells nothing and the line means the word — ウチ is うち, コイツ is
  こいつ. Never on a name, which is what keeps ココ a character.
- **Two dictionary authorities, and they answer different questions.** The
  master (Sankoku) says how a word is *spelt* and is the vocabulary scale; the
  `standard` role — 明鏡, 小学館 — says only what is *one word*
  (`SudachiTokenizer::segments` against `lexicon`). 意味ありげ, 何度, 図書室,
  被害者 and 270 more are words because 明鏡 lists them; それ is still それ
  because the master alone spells things. Merging the two was measured and is a
  disaster: 明鏡 lists the archaic kanji of every function word, and admitting
  those as spellings rewrote 5,064 lines into 其れ, 此の, 迚も. Three rules keep
  the standard dictionaries inside their remit — they may not respell a word in
  kanji the text did not use (今まで is not 今迄), may not license an expression
  opening on a function word (から目 ate 目を離す), and never enter the
  denominator, so a word only they list stays off the master scale.
- **A word no dictionary lists is spelt the way it was read.** Sudachi's
  normalisation is a guess there, and it guessed 御陰 for おかげ, 此奴 for
  コイツ, 切っ掛け for キッカケ, 紫恵 for シケイ — 121 lines of kanji the
  reader never saw. The fallback keeps the surface when normalising it would
  add a kanji that is not in it.
- **A compound the master doesn't list stays whole**, and **adjacent parts it
  lists as one word are rejoined** (`recompose`). Splitting such a compound into
  listed parts is what `decompose` used to do, and it was removed: it made 145
  sightings of 牢屋 out of 牢屋敷 against the 13 really read, cut 味方 into
  "taste" + "direction" and レイピア into レイ + ピア, while the two compounds it
  was written for had stopped reaching it — 懲罰房 because Sudachi calls it a
  place name, 医務室 because Sankoku lists it now. It destroyed words and
  produced fragments. An unlisted compound is a word the reader has not judged,
  and belongs in the ledger as one. Names are never rejoined.
- **A rejoined expression is matched with its last word in dictionary form**, so
  気になって and 気になる are one identity. Matching the surfaces alone could
  only ever find the uninflected sentence, which is why 気になる, 声をかける,
  口を開く and 158 more idioms Sankoku already lists sat split — 587 occurrences
  over the corpus, and the dictionary was never the obstacle. Three fences hold
  it, each one measured: the head may still not be a bound stem (続い + て must
  not spell 続いて), the conjugated word must be a content word (a conjugated
  auxiliary is the previous word's inflection — そう + な would spell そうだ),
  and **the run must begin on a content word**. That last one is what keeps
  と + し out of the listed とする; without it the quotative particle was
  swallowed on eight lines of the golden corpus.
- **A name is not vocabulary, and which words are names is a fact about the
  work.** `work_names` holds its cast — imported from VNDB by
  `jp-script names <work>`, extended by hand with `names <work> add` — and the
  tokenizer asks that list before it asks Sudachi. Sudachi's 固有名詞 is a
  per-occurrence tag and answers only for a name nobody listed; ingest's
  majority vote still folds that per _term_ over a whole pass.

  The cast list does three things no rule could. It **keeps a name whole**
  (`join_names`): 世凪 arrived as 世 + 凪 and both halves were counted, 2,412 and
  2,385 times over one script. It **takes apart what Sudachi glued to a name**
  (`split_names`): 凛と came back as the adverb 72 times, and the split is
  allowed only where no dictionary lists the whole and what is left over is
  grammar, so ウィルス and 出雲大社 stay whole. And it **spells the name as the
  text spelt it**, so すもも is not the fruit 李 and シャチ not the orca 鯱.

  **A cast name common enough to be an everyday word is the word.** VNDB lists 母
  as a character; it is rank 872 in fiction and the work writes it to mean a
  mother on nearly every page. `NAME_VETO_RANK` is 5,000, which also vetoes 唯,
  おじさん, 鏡, 南 and 翼; 凛 at 9,368 and 親方 at 19,076 are names. Where the
  frequency cannot decide — 看守 is 24,066th and is a prison guard 230 times in
  a VN set in a prison — `jp-script names <work> drop` records the judgement,
  under its own source so a refetch cannot undo it.

  **VNDB's aliases are prose as often as names.** "Prison guard", "Old man",
  "Magical Girl Riruru": splitting a romanized form on its spaces made guard,
  man, Old and Girl into people. `fetch_cast` splits only a Japanese form, and
  `work_names::all` hands the tokenizer nothing that has no Japanese in it —
  a romanized name is not what a Japanese script writes, and can only collide
  with English the text really contains.

  What is left is the mirror error, a common noun SudachiDict tags 固有名詞.
  `ordinary_headword`'s mixed-script rule catches it wherever the term carries
  okurigana, since a Japanese name does not; where it carries none — 眸, 王子,
  城, 金, 鏡, 悪魔, 予定調和 — nothing structural separates it from 橘 or 葵 and
  `NOT_A_NAME` names them one at a time. A work that really does have a
  character called 悪魔 says so in its cast list, which is asked first.

  Still open: a work's spelling of the pronouns — あてぃし, わたくしめ, ぼくちん
  — which are a character's voice and not vocabulary. Blacklisting in triage is
  what to do meanwhile.

### Fixing one

A wrong token is diagnosed and repaired in a fixed order. Skipping the first
two steps is how a rule gets changed for a case that turned out to be something
else.

1. **Ask the pipeline what it did.** `#tokenize` (⚙ → `#tokenize`) renders
   `jp_core::tokenize::trace` for the line: the Mode C segmentation, the gate's
   verdict per morpheme, every join offered and why it was refused, and which
   rung of the identity ladder named the term. The trace is the real
   implementation talking, not a reconstruction, so the step that is wrong is
   the code that is wrong.
2. **Find out how often it happens.** `jp-core/examples/tokens.rs` dumps every
   line's tokens; `examples/joins.rs` lists what a wider join rule would do.
   Both read `knowledge.db` directly, so take a snapshot first rather than
   reading the live one under a session:

   ```sh
   sqlite3 ~/.local/share/jp-tools/knowledge.db ".backup /tmp/k.db"
   cargo run --release --example tokens -p jp-core -- /tmp/k.db system_full.dic > before.tsv
   # ...change the rule...
   cargo run --release --example tokens -p jp-core -- /tmp/k.db system_full.dic > after.tsv
   diff <(cut -f2 before.tsv) <(cut -f2 after.tsv) | head -50
   ```

   **Build the "before" from `HEAD` in a `git worktree`**, not from an earlier
   dump. These tools read the live `knowledge.db`, which grows while you work,
   and they have drifted from production before — `tokens.rs` went a year
   without the reader ranks, so what it printed was the pipeline with the
   short-kana guard switched off. Three env toggles exist so one rule can be
   diffed against itself with everything else held still: `TOKENS_NAMES=off`,
   `AUDIT_CAST=off`, `AUDIT_GUARD=off`.

   **A rule change is judged on that diff, not on the case that prompted it.**
   Every rule here trades one mistake for another, and the trade is only
   visible over the corpus: と + し → とする looked like an improvement in
   isolation and swallowed the quotative particle on eight lines. Two rules
   this week were built, measured, and taken back out again on the strength of
   it — see *What was refused* in `jp-core/PARSE-DEFECTS.md`.
3. **Reach for the narrowest knob that fits.**

   | the error | the knob |
   | --- | --- |
   | one string joins wrongly (思いで, ものとする) | `NEVER_JOIN` in `tokenize.rs` — one reviewed judgement per string, and it cannot cost anything not named |
   | Sudachi's boundary runs through a word it lacks (なん + てひどい) | `CUT_BEFORE_AND_AFTER` — the line is handed over in pieces, and only where the analysis shows the boundary came out wrong |
   | a standard dictionary is licensing nonsense in bulk | `jp-dict set-role <id> reference` backs it out entirely; nothing else changes |
   | a name is being counted as vocabulary | `jp-script names <work> add <name>` — the cast list is asked before Sudachi's tag and is remembered per term |
   | an ordinary word is being dropped as a name | `NOT_A_NAME` in `tokenize.rs`, once `ordinary_headword`'s mixed-script rule has been ruled out |
   | a word should be kept whole | mine it: the mined deck is the tokenizer's second wordhood source, so tomorrow's lines keep it |
   | anything structural | a rule in `join_run` or `identity_ladder`, with the golden fixture read line by line |

4. **Run the golden test and read its diff.** It fails on any change and that is
   the point:

   ```sh
   JP_TOOLS_SUDACHI_DICT_PATH=$PWD/system_full.dic \
     cargo test -p jp-core --features test-support -- --ignored
   ```

   Regenerate only once the diff is the change you meant, and **pass the
   existing fixture corpus back in** so the sample does not reshuffle:

   ```sh
   cargo run --release --example golden -p jp-core --features test-support -- \
     ~/.local/share/jp-tools/knowledge.db jp-core/tests/golden/corpus.txt system_full.dic
   ```

5. **Re-derive the ledger.** `POST /api/vocab/rebuild` re-ingests every line
   under the new rules, carries stranded judgements onto their new keys and
   prunes what the pass no longer produces. Nothing is lost by running it
   twice; it is the undo for a tokenizer change as much as the commit of one.

### Known errors, and why they are left alone

A random audit of 240 tokens puts content-word accuracy near 97%. What is left
is understood, and each was measured and declined rather than missed. **A
Sudachi user dictionary is the one mechanism that would take all of them**: they
are all cases where Sudachi's lexicon lacks the word, and every repair further
down the pipeline is either a hand-list or a loosening that costs more than it
buys.

What that dictionary would cost, measured (`jp-core/examples/missing.rs` counts
it): 13,834 of Sankoku's 81,884 headwords do not survive Mode C as one morpheme.
Only ~7,363 belong in a segmenter — 2–3 morphemes, containing kanji, no particle
or auxiliary inside. The rest are phrases and idioms (ああ言えばこう言う,
あがきが取れない), and putting those in the lexicon makes Sudachi merge them
wherever their morphemes co-occur; they are recomposition's expression path,
which has structural guards a Viterbi cost does not. Two things make it work
rather than backfire: **sudachi.rs's `ubuild` has no cost estimation** — unlike
the Java tooling, `left_id`, `right_id` and `cost` are required `i16`
(`dic/build/lexicon.rs`), so each entry needs them cribbed from an exemplar
system entry of the same POS — and Sankoku carries no POS, its `rules` column
being empty for 82,271 of its rows, so POS has to come from the final morpheme's
Sudachi tag. It also moves wordhood into costs `#tokenize` cannot explain.

**Matching master headwords against the raw line before Sudachi sees it was
measured and rejected.** It is the obvious way to protect a word the segmenter
lacks, and over this corpus it protects the wrong things: at a four-character
floor it freezes じゃない (322), しまった (143), どうして (72), のだろう (71),
いけない (62), にとって (60) and 分からない (56) into single tokens. Those are
Sankoku headwords, so no dictionary check rejects them, but as ledger rows they
are grammar and they stop しまった counting as しまう and 分からない as 分かる.
A further 478 matches cross a token boundary outright — ダイイン, 女になる,
ってんだ. Raising the floor to five characters cuts the yield to 463
occurrences and does not change the shape of what is caught. Longest-match has
no way to tell a word from a construction; recomposition's expression path
already does, which is why the length cap was the right thing to widen instead.

- **待ちたまえ → 待ち + た + まえ**, and まえ becomes the noun 前. たまえ is 給え,
  which Sankoku lists, so `recompose` could rejoin on the reading — but that
  fence must stay shut. 3,016 adjacent pairs in the corpus have readings
  spelling exactly one master headword, and they are て + いく → テイク,
  ない + ん → ナイン, は + ない → 派内, いる + か → 海豚. Five tokens of gain
  against three thousand ways to be wrong.
- **皆守 → 皆 + 守**, 191 lines of 素晴らしき日々 — and now fixable rather than
  known: it is the case `work_names` exists for. `jp-script names <work>` then
  `POST /api/vocab/rebuild` re-derives what was counted.
- **擦る is する.** Sudachi gives the 五段 verb both the dictionary form and the
  normalised form する, identical to the irregular. Only the conjugation class
  separates them, and using it means narrowing the kana exemption in
  `conjugatable_lemma`, which exists for the auxiliaries. 12 tokens.
- **ない/無い and こと/事 are two ledger rows each.** Sudachi's dictionary
  normalises いう→言う and できる→出来る but not these. No derivable rule fixes
  it: "merge a kana headword with the kanji one that reads the same" also merges
  た with 他.
- **A stammer with no comma is not caught** — 「そ……そう」, 「そそそう」. The
  rule keys on the comma because that is the part that is unambiguous.
- **他 is both た and ほか**, 65 and 66 encounters, and neither is a double
  count: bare 他 occurs 149 times and Sudachi assigns each occurrence one
  reading. Both are Sankoku pairs, so nothing downstream can arbitrate. This is
  the same case the ない/無い entry above names as unfixable.
- **The headword shown is the canonical spelling, not the written one.**
  傍 normalises onto 側/そば, which is the mechanism that makes いう and 言う one
  row. `term_surfaces` keeps what the text actually wrote, and the triage UI
  shows it beside the count — that breakdown, not the headword, is the record of
  how a word was spelt.
- **Function-word counts carry roughly 10% noise** from stammer fragments,
  onomatopoeia and garbled hook output landing on だ, に, の. Harmless for
  vocabulary; do not build a grammar metric on particle frequencies.
- **Anything the master dictionary lists is a word, whatever its tag.**
  `counts_as_word` admits a content word, or any token whose
  `(headword, reading)` pair Sankoku lists. The gate decides what gets a row;
  `COUNTS_AS_VOCAB` decides what gets counted.
- **A re-tokenization strands judgements, and the rebuild re-homes them.**
  `carry_stranded_judgements` moves a status to whatever the term is called now,
  never over the target's own assertion.

The reading view:

- **A tap in the feed judges the word under it.** Two states: anything marked
  becomes `known`, a word already known becomes `unknown`. `new` and `seen` are
  unreachable by hand and must stay that way. No undo and no toast — the mark is
  the report, and a failed write is the mark coming back. It is hit-tested with
  `caretPositionFromPoint`, and **nothing in the feed is made clickable**: an
  interactive layer would sit between the reader and the text Yomitan scans.
- **The live badge reports the writer, not the connection.** vn-ws-logger.py
  publishes `settings.vn_logger_heartbeat` (its Textractor WS state and its
  unwritten backlog) and the SSE stream republishes a verdict every 2s. The
  badge was once the `EventSource` alone, which sat on "live" through three
  hours of capturing nothing: this stream is healthy whenever read-stats is up,
  and knows nothing about the two hops in front of it.
- **Marks are drawn, never markup.** `jp_core::highlight` sends offsets
  per line and `paintMarks` draws a rectangle per word into a layer _behind_ the
  text. Yomitan scans this DOM, so one text node per line is a constraint.
  Offsets are UTF-16 code units because that is what a `Range` indexes in.
  Three tiers are painted and `known` is not one of them — the absence of a mark
  is what makes the marks readable — but a `known` span is still sent, since a
  span is also the region a tap judges.
- **A common word not known is underlined on top of its tint.** Each span
  carries its jiten rank and the client underlines `new`/`unknown` at or under
  `reader_common_max_freq_rank` — not knowing a rare word is expected, not
  knowing a common one is the gap worth seeing. The threshold is applied in the
  client, so changing it repaints what is already on screen; an unranked word is
  never underlined, since that is the case where the claim cannot be made. The
  ranks are preloaded into the `Highlighter` for the master headwords — a
  `dictionary_frequency` query per word would sit on the path that draws a line
  as it is being read.
- **The feed re-pins to the bottom on a new _line_, not on a new `lines`** —
  judging a word rebuilds the array without adding to it, and an id-keyed pin
  kept yanking the word out from under the finger. A reflow re-pins too, on a
  height test (`pinToBottom`), because the web font, a page of history and a
  resize all move the feed under a reader who never touched it.
- **The `◌ marked` filter is a view, never a write.** It filters on membership
  (`keptIds`), not a live predicate, or judging the last marked word in a line
  deletes that line from under the finger. `lines` stays the whole feed and the
  filter applies at the last moment, so everything that measures or hit-tests
  text takes `visible` — and the repaint must depend on `keptIds`, not only on
  `lines`, or a backscroll strands every mark a page-height off its word.

Mining:

- **Mining is implicit.** Every card path — the overlay's `reader/mine`, and
  Yomitan's `addNote` through `routes/ankiproxy` — adds through
  `services::card::add_note`, which fires vn-capture.sh once Anki accepts the
  note. There is no mine button, and the overlay has no popup button either — a
  side mouse button mines the word under the pointer, and another judges it.
- **The popup can overrule the tokenizer about a position.** It scans the raw
  line from the clicked word rightwards (`reader/define::expand`) and shows a
  chip per `(term, reading)` a dictionary holds for a prefix of it, longest
  first; picking one re-opens the popup on it and ✓ ✗ ＋ then act on that term.
  The scan runs beside the definition rather than behind a button, because the
  row has to know whether there is anything to offer before it draws — on most
  words there is not, and it draws nothing. Two failures, one answer: 経年劣化 is a Jitendex headword and not a
  Sankoku one, so its two halves are both right and no rule joins them, and
  素振り is そぶり or すぶり and the tokenizer picks one. Both are the reader
  seeing what the pipeline cannot.
  - **Two kinds of candidate, and the second is the point.** A literal prefix
    of the line finds a compound the tokenizer split (経年劣化). A prefix that
    ends on a token boundary with its last token put back in canonical form
    (`Highlighter::prefix_forms`) finds an expression the sentence conjugated —
    しびれを切らした is しびれを切らす in every dictionary that lists it, and no
    literal prefix of the line spells that. Expressions are most of what a
    dictionary holds and the tokenizer cannot join, so literal-only found
    almost none of them.
  - **The scan also offers the other kana alphabet.** アレ is a Jitendex
    redirect and Sankoku has no entry for it, so the popup opened on a
    cross-reference; the hiragana spelling of every candidate is offered
    beside it. The tokenizer folds too now — the objection was that 424 of the
    549 encounters were ココ, a character in the VN, and the cast list answers
    that.
  - **A candidate carries its ledger key beside its spelling**, resolved
    through the shared `Highlighter` — a dictionary headword is text from
    outside the tokenizer, and Jitendex's 素振 would otherwise become a second
    ledger row beside 素振り. The popup defines and shows the dictionary's
    spelling; judge, mine and the duplicate check all send the key.
- **The mined badge asks Anki, not `anki_notes`.** The table is a snapshot taken
  on demand, and the case that matters is a card made seconds ago;
  `reader/mined` runs the same duplicate check Yomitan does. It is fetched
  *after* the definition renders so a shut Anki cannot delay the answer being
  asked for, and `reader/mine` returns the new note id so a mine raises the
  badge on an open popup without a second query.
- **A lookup is the popup opening, and nothing else is one.** `reader/define` is
  the overlay's whole lookup path, so it records; judging and mining from the
  side buttons go nowhere near it. Reaching those two through the popup made
  every judgement look like a word that had to be looked up, which is the one
  number the lookup tax is measured from.
- **The popup carries those actions as buttons too, and retracts what they
  cost.** Not every way of reading the overlay has side mouse buttons — driving
  the PC's mouse from a phone has none — so ✓ / ✗ / ＋ sit in the popup head as
  well. Marking a word `known` there posts
  `reader/define::retract` with the id `define` returned, which **deletes** that
  row. Deleted and not flagged on purpose: every figure over `lookups` is
  derived from the rows at query time, so a row that is gone is gone from all of
  them, while a `retracted` column would keep counting in whichever reader
  forgot to filter it. The id is paired with the term in the delete, so a stale
  id cannot take out an unrelated row, and only `known` retracts — `unknown` and
  a mine both mean the definition was read. **A lookup is presence evidence too,
  so the delete leaves a `reader_marks` row at the lookup's own timestamp**:
  the popup was not a lookup, but the reader was demonstrably at the screen, and
  without the mark the surrounding gap would quietly stop counting as reading.
- **`VocabDefFull` carries Yomitan's per-dictionary wrappers**, not just the
  glossary: the note type styles `.dict-<slug>-title` and `.dict-<slug>-body`,
  and `.dict-jitendex-body > div > ol > li` is what hides Jitendex's star and
  its ① ② numbering.
- **The card's dictionaries and the popup's are two lists.** `CARD_DICTIONARIES`
  is Sankoku and Jitendex, because those are the two the note type has CSS for
  and a third would land on the card unstyled. The popup shows everything
  installed and opens on the master, with `jp_core::define::SECOND_PAGE` next. Adding a dictionary changes the
  popup; it changes the card only when the note type gets a rule for it.
- **The class name is fixed per dictionary, never derived from its title.**
  `CARD_DICTIONARIES` pairs a title prefix with the class (`sanseido`,
  `jitendex`), because both titles carry a version the release moves — Sankoku's
  edition, and Jitendex's date in Yomitan's own copy of it
  (`Jitendex.org [2026-02-05]`). A slug built from the title stops matching on
  the next update, and the star and ① ② rules are written against
  `.dict-jitendex-body` alone, so the block would come back unstyled with the
  field still looking full.
- **A capture is anchored at the add, not at the capture.** `card::add_note`
  stamps `now_ts()` before forwarding and passes it as `VN_ANCHOR_TS`. Nothing may
  be awaited in front of the capture: in `enrich_added_note` the CompactDef call
  runs _alongside_ it (`tokio::join!`) with its Anki write after. The two
  `updateNoteFields` stay strictly ordered.
- **An accepted Anki write is not a stored value.** If the note is open in
  Anki's editor, the editor's next save overwrites the field with nothing
  logged. The CompactDef path uses `anki::update_note_field_verified`, which
  reads the field back. It does not retry — don't open a freshly mined card for
  a few seconds.
- **The chime is the only report a mine gets.** `services::chime::mine_complete`
  plays only when the capture reported `ok` _and_ the CompactDef write verified.
  Keep it that strict: silence is the signal to check the log.
- **The audio window's next-line bound is a hard cut, and that is a known
  defect.** When the next line is unvoiced the previous voice legitimately
  plays past its timestamp and the clip is truncated. It shipped that way
  because a truncated clip of the right line beats a whole clip of the wrong
  one. Replacing the rule needs measurement first — how closely a voiceline's
  onset tracks the hook, and how well a line's mora count predicts its
  duration, over a real session rather than a menu. A script that did that
  lived here and was deleted unused; if it comes back, two traps it already
  fell into: letting every line search independently, so unvoiced lines claim
  the next line's voice and invent rates like 131 morae/s (the tell is
  duplicate `dur` on neighbouring rows), and selecting the sample by
  `|onset| < 1.0` before reporting that onsets fall within 1.0.
- **CompactDef is told the surface, never the headword.** The tag axes rate the
  spelling the reader met, so the prompt gets the `<b>` span out of the sentence
  field (`anki::bolded_span`) and the vocab field only as a fallback. Measured on
  one sentence: すえた comes back UNCOMMON · PLAIN and 饐えた RARE · LITERARY —
  the headword prices its kanji, and a phrase people say gets tagged as if it
  were literary. Withholding it costs the model the word's identity, which it has
  to infer from the sentence; that is the accepted trade, and the reason the
  sentence keeps its bold markers.
- **The card report reads Anki's review log, never `cardsInfo`'s `mod`.**
  `stats::card_evidence` sorts each mined card by what the reading says about
  it since its last review — met without a lookup, looked up on a long
  interval. `mod` was the obvious cutoff and is wrong: something had bulk-touched
  this collection, so 2,196 of 2,256 cards read as "reviewed 9 days ago".
  `anki::fetch_deck_reviews` takes the whole deck's log in one `cardReviews`
  call (~20k rows, well under a second) and `mod` is only the fallback for a
  card with no log. `GET /api/anki/cards` **writes nothing** — it reports what a
  sweep would act on, and the thresholds are a guess until the buckets have been
  read against words already judged known.
- **Note ids are epoch milliseconds**, so they double as card creation times.
- **Only engagement actions leave `reader_marks`.** Explain does; clear does
  not. A retracted lookup leaves one in place of the row it deleted, which is
  the only writer that backdates a mark rather than stamping `now`.

## Working on it

Don't restart the live stack or touch `~/.local/share/jp-tools` while a VN is
being read. Use an isolated instance:

```sh
scripts/dev-instance.sh run             # :3299, on a frozen copy of the data
scripts/dev-instance.sh snapshot before # record every endpoint
# ...make the change...
scripts/dev-instance.sh check before    # must print IDENTICAL
scripts/dev-instance.sh browser         # the SPA actually renders
```

For a refactor that must not change behaviour, the snapshot diff is the proof.
The browser check exists because the client is unbundled ES modules loaded
straight from disk: a bad import path renders _nothing at all_ while every JSON
endpoint still passes.

`run` holds the terminal and has no `stop`, so a backgrounded instance outlives
the session. Take a free port (`DEV_PORT=3298`) rather than clearing it.
**Never `pkill -f` your way out of that** — the dev instance and the live :3200
service are the same binary path. Resolve the PID from the port instead
(`ss -ltnp | grep :3299`).

```sh
cargo test -p read-stats     # unit + integration (tests/api.rs)
```

`tests/api.rs` runs the real router against a throwaway database — the layer to
add to when the question is "does the SQL select what the derivation assumes".

## Frontend notes

- Preact + htm from a CDN import map, no build step. `charts.js` and
  `style.css` are re-export/`@import` facades — add a chart or a sheet there.
- **Never let literal text and `${...}` straddle a line break inside an `html`
  template.** htm collapses the whitespace, which silently rendered
  `snapshot 0 min ago` as `snapshot0 minago`. Build the string in JS and
  interpolate it whole.
- The dashboard polls once and passes the result down — half the cards are
  different readings of the same days. **Tabs choose what renders, never what is
  fetched.** `/api/kanji` is the one exception, and only because no other panel
  reads it: it walks every line ever read, so the kanji tab fetches it itself
  rather than holding up the first paint of a page not showing it.
- Five tabs, one per question: **Today** (`current-reading.js` over `day.js`),
  **Trends** (one range selector over every chart), **Library**, **Kanji**,
  **Vocab**. `#settings` and `#tokenize` are reached from ⚙ and render inside
  the shell like any panel; `#read` is its own route and unmounts the dashboard.
- **Pause capture appears only where reading does** — in `#read`, and under ⚙
  beside the settings. A live switch next to numbers that only report was read
  as a filter.
- **Library has two levels.** The shelf lists works as cards; opening one
  replaces the tab with `work-detail.js` over `GET /api/works/detail`, keyed by
  title. A work with no reading behind it does not appear. Logged articles
  collapse into one `Articles` row (`stats::work::ARTICLES_WORK`). The log form
  has two modes over one POST: _pages_ estimates chars from a page count,
  _paste text_ counts the article exactly, via `/api/text/count` rather than a
  `length` in JS.
- **`#tokenize` reports the tokenizer, not the ledger's folding.**
  `Analyzed.reading` is the reading the token was produced with; where the
  status came from a different row, `judged_as` carries that row's reading in
  its own column. The feed folds them in `spans`, on the way out of `analyze`.
  The page writes nothing — no ledger row, no count, no presence mark.
- **The `why` card is the tokenizer's own trace, not a reconstruction of it.**
  `jp_core::tokenize::trace` is threaded through the real predicates and
  `SudachiTokenizer::explain` is `tokenize` with the recorder on, so a step is a
  line of the pipeline. An explanation derived by a second implementation would
  drift from the thing it explains, and would be worth less than nothing on the
  day it mattered; `explaining_a_line_yields_the_tokens_tokenizing_it_does` is
  what holds the two together. Recording is inert when off — `Trace::push` takes
  a closure — so ingest pays a bool check per decision and no `format!`.
- **The trace defaults to the decisions with a fork in them** (`decisive` in
  `panels/tokenize.js`). A full line is ~60 steps and most are the pipeline
  agreeing with itself: recomposition offers every run of two and three adjacent
  tokens at every position and nearly all spell nothing, every particle gets a
  gate that keeps it, and punctuation falls down the whole identity ladder every
  time. Those are true and they are not why anything happened. What is left is a
  rewrite, a stammer drop, a join taken or refused, a split, and an identity
  that had more than one candidate — 綺麗 → 奇麗 is a fork, を → を is not.
  `ROUTINE_IDENTITY` there is the one string the two languages share: it must
  stay character-identical to the rung `identity_ladder` returns for a plainly
  listed pair, or the default view silently fills up with every particle.
- **A bulk write shows its rows first.** `blacklist-non-words` judges rows the
  queue never displays, so `GET /api/vocab/non-words` lists them and the button
  only appears once they are on screen.
- **Triage ticks on two signals, never one** (`vocabulary::preselects_known`): a
  word is preselected `known` only if it was met at least
  `triage_min_encounters` times **and was never looked up**. Unticked means
  `unknown` on submit, so a one-signal default would write wrong assertions in
  bulk. The rule lives server-side because it decides what gets written.
- **The sweep is scoped to what has been read since the last one.**
  `sweep_through_ts` is compared against `vocabulary.last_seen`. It moves
  **on submit, never on load**; only for a
  request that asked (`advance_sweep`); and it is a filter and nothing else —
  `scoped=0` still reaches every ready row.
- **The sweep's two orderings are one batch, seen from either end.**
  `order=frequency` sorts the same rows by jiten rank instead of encounter
  count, so the page reaches words common in Japanese rather than common in
  what was read. It changes nothing about the filter, the counts or what a
  submit writes; an unranked word sorts last rather than dropping out.
- **A rule the UI needs is a tooltip, not a paragraph.** Prose that explains
  what a number means goes in `title=` on the heading or tile it explains. Text
  on the page itself carries data — a count, a range, a date.
- **Status colour is one scale, in HSL, in `base.css`.** Hue names the status
  (211 blue `new` / 276 violet `seen` / 28 amber `unknown`), lightness says how
  loudly, and the dark ramp mirrors the light one. Both places that show a
  status read these, so the tint under a word in the feed is the colour of the
  pile it is counted in.
- Selected state has one vocabulary: `background: var(--meter-track)` with
  primary ink (`.segment-on`, `.toggle-on`, `.tab-on`). `--series-1` at full
  strength is spent on the paused alarm alone.
