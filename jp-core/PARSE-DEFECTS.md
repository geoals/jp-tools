# Parse defects

Words noticed misparsed while reading, worked through in batches. **Grouped by
what the pipeline does wrong, not by the word that found it** — a word is a
sighting, and a sighting is only worth keeping because a fix is judged against
it.

**Six mechanisms open**, five checked against the live pipeline on 2026-08-16
and the sixth on 2026-08-19, ordered by what a fix is worth. Three further
questions are about the vocabulary denominator rather than the parse and are
kept apart from them; a fourth — whether the reader's lookup surface should be
the ledger's key at all — has its own section. What has been fixed is one line each at the bottom.

**The standard this list is worked to: as many whole lines as possible where
every token is the right headword — and where the pipeline is unsure, no match
beats a wrong one.** A token left as written costs a lookup; a token keyed on a
word nobody read is a false assertion that spreads, into the ledger, the popup,
the count and the triage queue. The `drops_kanji` rule, the two-mora guard, the
ambiguous-reading refusal and the noise gate are all that principle. So are the
trades taken on そこで, ときに and 手を入れる: each cost a handful of real
matches — 7, 3 and 1 — to stop 48, 32 and 8 wrong ones, and what is left in
their place is the plain words, which are still words.

**Read *Picking this up cold* first if the tools are not already built**, then
*Next*. Next ranks what is left, says what has already been measured so it is
not measured twice, carries a *What has been tried and measured wrong* list, and
names the one change waiting on a decision rather than on work.

---

# Picking this up cold

Everything below is measured against the corpus rather than argued, so the first
move is always to rebuild the working set. None of it lives in the repo.

**1. Freeze the database.** These tools read the live `knowledge.db`, and it
moves while the reader reads.

```
sqlite3 ~/.local/share/kotodex/knowledge.db ".backup /tmp/snap.db"
```

**2. Dump the corpus, before and after.** One line per line read: the text, then
`surface/headword/reading` per token, tab separated.

```
cargo run --release --example tokens -p jp-core -- /tmp/snap.db system_full.dic > /tmp/before.txt
```

A clean before/after needs the baseline built from **`HEAD` in a `git worktree`**,
because a rebuild of the same tree gives the new behaviour on both sides:

```
git worktree add /tmp/base HEAD
```

Diff the two dumps by token, not by line — the useful summary is which
`surface/headword` pairs disappeared and what replaced them, counted.

**3. The three env toggles** switch one rule off so it can be diffed against
itself: `TOKENS_NAMES=off`, `AUDIT_CAST=off`, `AUDIT_GUARD=off`.

**4. One line, with the reasoning**, through `#tokenize` or:

```
curl -s localhost:3200/api/tokenize -H 'content-type: application/json' \
  -d '{"text":"…"}'
```

The trace is the fastest way to find *why* a token came out wrong — it names the
rung or the fence, and guessing at that has been wrong twice (でも was refused by
the length floor, not the content-word fence; たまえ by the lattice rather than
by any rule, and its recorded cause was wrong twice over).

**The two audits worth repeating**, and the reason to repeat both:

- `cargo run --release --example joined -p jp-core -- /tmp/snap.db system_full.dic 3`
  — every expression `recompose` actually built, grouped by result, marked `*`
  where the run is a content word followed by nothing but grammar, with the
  lines it was built on. This is what finds a join firing where it should not.
- **Uniform lines read as sentences.** Draw ~150 lines from the dump, print the
  line and its tokens, and judge each token against what the sentence meant.
  This is what finds everything else; a token draw cannot, see *Where the value
  is not*.

**Not live.** The tokenizer changes below are committed but not running:
`scripts/start-all.sh restart read-stats` picks them up and
`POST /api/vocab/rebuild` re-derives the ledger under them. Neither is safe to
do mid-session while a VN is being read.

---

# Next

Ranked, with what has already been measured so it is not measured twice. The
procedure for any of them is read-stats' CLAUDE.md under *Fixing one*; the
addition worth knowing is that a clean before/after needs a **baseline built
from `HEAD` in a `git worktree`**, because several of these tools take their
inputs from the live `knowledge.db` and it moves under you. Three env toggles
exist so a rule can be diffed against itself: `TOKENS_NAMES=off`,
`AUDIT_CAST=off`, `AUDIT_GUARD=off`.

## Waiting on a decision, not on work

**One mora of kana is never a word of its own, whatever a dictionary says.** The
one change in this file that needs an answer rather than a pass: it would take
46 ledger rows and 1,378 encounters, it **overrules a dictionary** where no
other rule here does, and it has to drive the reader's paint as well as the
ledger to stay coherent. The argument and the numbers are under *Open*, group 1.

## Then, in value order

Done since this section was last written, all under *Fixed*: the katakana fold,
the colloquial adjective ending, the join's clause-initial list, `drops_kanji`,
ないと before a quoting verb, そこで / 中には / ときに / 手を入れる, the inflected
half of the kanji swap, ちゃんと / ものの, and そういう over そういえば.

**The join list is finished.** ないか (415) and ないで (178) were measured with
the last two and are **right**: 「じゃないか」 is the negative question and
「言わないでください」 the negative te-form, which is what those entries are. Do
not re-measure them.

The open groups are already in value order, so *Open* is the queue. What is
worth saying here is what separates them:

1. **Group 1 is the whole game** — ~1,400 encounters against ~370 for the other
   four combined, and every one of them a false ledger row rather than a missed
   word. Three quarters of it needs no decision; the one-mora rule does.
2. **Group 2 is a decision, not code**, and the same one the denominator section
   asks. Until it is made, the headline vocabulary number has an unstated policy
   inside it.
3. **Groups 3 and 5 cost a lookup, not an assertion** — a word left in pieces is
   findable, a word keyed on the wrong headword is not. That is why 110 tokens
   of refused joins rank under 220 tokens of orthography.
4. **Group 4 has the most sightings and the least weight**: eight of them, thirty
   tokens, one structural cause. Worth doing as one batch on a day when the tools
   are already rebuilt, not one at a time.

## What has been tried and measured wrong

Do not build these; each cost a pass to disprove.

- **A general clause-initial join rule.** Admitting every clause-initial two-kana
  master headword fires 1,975 times over 76 strings — 何か, 何が, 何を, では,
  だと, して — because a clause opening on a pronoun and a particle is just a
  sentence starting. And refusing every mid-clause expression takes 本当に,
  ために and すぐに. Both directions are named lists.
- **Widening the reading join to reach an all-kana expression.** そういえば,
  どうしても, それにしても and ただでさえ are Sankoku headwords whose surfaces
  spell nothing and whose parts include function words, so no path reaches them.
  Admitting an all-kana run of five morae or more that opens on a content word
  builds **927 changes over 886 lines**, and they divide three ways: ~300 real
  expressions, ~150 grammar points called words (ついている, なっていない,
  しまったら, どうしたら, ならなかっ), and the rest a wholesale orthography
  decision the ledger has never made — わからない keyed on 分からない 177 times,
  ありえない on あり得ない 45, すみません on 済みません 21.

  Fencing the two off is what kills it: **requiring no inflected part and no
  kanji the surfaces lack leaves exactly zero tokens**. Every expression this
  path could reach either holds an inflection (どうし + て + も, もしか + して)
  or is a kana rendering of a kanji headword. There is nothing in between, so
  the path cannot be opened a crack — see `NEVER_BEFORE_A_CONDITIONAL` for what
  was done instead.
- **Three ways of tightening the reading fallback**, all of which cost more than
  they save. The rung hands a short kana surface a kanji headword by sound
  alone, and the false rows it produces (そん → 村, びくん → 微醺) look like a
  fence is missing. All three attempts hit the same wall: **the lever is the
  spelling-class lever**, and the good half of this rung is the spelling class
  working.

  - *Refusing any kanji headword to a hiragana surface of three morae or less
    at that rung, at any rank* — **1,488 tokens**, and the losses outnumber the
    wins ten to one: いつ off 何時 (155), いつも off 何時も (152), やめ off 止める
    (116), おじ off 伯父 (119), さっき off 先 (105), におい off 匂い.
  - *Raising `SHORT_KANA_MORAE` from two to three*, so the existing rarity fence
    reaches びくん — **130 tokens**, and it takes 鼬, 欅, 襖, 蕾, 盥, 蠍, 庇 and
    胡散 off their spellings. The file already predicted this: three morae is
    where the coincidence stops and the evidence starts, ほうき is 箒 at rank
    22,217 and is right, and the guard stops at two for that reason.
  - *Extending the interjection rule to 副詞 as well* — **250 tokens**, and 副詞
    is the tag a real mimetic carries: it takes おずおず off 怖ず怖ず, ひんやり
    off 冷んやり, まるっきり, つやつや, ふつふつ and ごうごう with it. Past two
    morae an interjection is a word too — おはよう is お早う and おかえり お帰り.
- **A rule about と before a quoting verb.** さらりと言った and ぴしゃりと言った
  are joins ending in と before 言う and the と belongs to the adverb.
- **Admitting the standard dictionaries' empty-reading headwords.** A Yomitan
  build stores no reading for a headword that is already kana, and the SQL in
  `dictionaries::standard_entries` filters those rows out — so 25,677 kana
  headwords never reach the segmentation authority at all, とはいえ and たまえ
  among them.

  **The first measurement of this was run wrong** and is corrected here: it
  removed the skip in `with_standard`, which sits *downstream* of the SQL
  filter, so it changed zero tokens and the hole was recorded as dead code. It
  is not. Removing the filter itself and letting a kana headword read as its own
  spelling moves **1,997 tokens over 1,926 lines**, and the half that matters is
  grammar made into words: それを 201, ないん 126, あると 76, いるか 66,
  わけない 56, ことになる 42, ことができる, ことがある, ことはない, ないし,
  ところを. Those are the phrase entries 明鏡 and 小学館 carry, and admitting
  them is the disaster the expression path's fences exist for. The real words it
  would recover — おじさん 113, そんなに 81, なんとか 63, ぽつりと, いくつか,
  くせに — do not pay for that.

  **And it does not even reach とはいえ**, which needs the function-word fence
  moved as well; see the entry below.
- **Letting a standard dictionary license an expression that opens on a function
  word.** The other half of とはいえ. Dropping `opens_on_a_word` from
  `expression_admitted` moves 80 tokens and 明鏡's から目 fires 13 times,
  destroying 目を離す, 目を逸らす, 目を背ける and 目を瞑る; から口 and 類がない
  come with it. とはいえ does not appear at all, because it needs the empty-reading
  filter gone too — so the two changes are only bad together.
- **Arbitrating a spelling by how much its reading resembles the surface**, and
  **taking the master's single reading whenever Sudachi's is unlisted** — see
  *What was refused* under the name batch.

## Where the value is *not*

**Identity defects were at diminishing returns and joins were not — and the
join work has now been done, so that is changing again.** What each pass moved,
for comparing the next one against:

| pass | tokens |
| --- | --- |
| the name batch | ~9,000 |
| `CLAUSE_INITIAL_ONLY` (でも, だが, ところで, すると, それで) | 1,807 |
| the master's other spelling, for a stem | 429 |
| `drops_kanji` | 406 |
| the noise rule | 338 |
| the katakana fold | 187 |
| a two-mora cry given a kanji word | 162 |
| 確かに joined, たまえ cut | 66 |
| ないと before a quoting verb | 120 |
| ちゃんと after a name, ものの before a noun | 85 |
| 中には, ときに, 手を入れる | 84 |
| そういう over そういえば | 70 |
| そこで | 57 |
| the うるさい family, うっさい with it | 20 |

Every *identity* defect still open is worth ones and tens — 牛乳粥 was 47, the
spelling class ~60, かたや and いやがおうにも 1 each. The inflected kanji swap
was the last one worth hundreds.

**Those draws are why the join class went unseen, and the reason is worth
keeping.** Judging a token against its line asks "is this word right", and a
wrongly-built ところで *is* a word — it is the wrong one for that sentence, and
it reads as fine unless the line is read as a sentence rather than as a bag of
tokens. A token draw is also weighted the wrong way: half of it is だ, た, は
and を, which are never wrong, so 1-in-60 by token is 1-in-3 by *line*.

Two passes found everything above, and both are worth repeating. Sampling the
joins themselves, grouped by what was built and shown with the lines they were
built on (`examples/joined.rs`). And reading 160 uniform lines as sentences,
which put the rate at ~70 of 2,499 tokens and turned up six classes the join
sweep could not see.

The denominator decision is still worth more than the tail of this list.

---

# Open

**Five mechanisms, not twenty-two words.** Each section is one thing the
pipeline does wrong; the words under it are the sightings that found it, kept
because a fix is judged against them. Ordered by what a fix is worth, which is
encounters weighted by whether the cost is a false ledger row or a missed one.

## 1. Wordhood judges Sudachi's normalisation, not what the reader saw

**~1,400 encounters, all of them false assertions.** `Wordhood::is_noise`
refuses a short kana term nothing lists — but it asks the dictionaries about the
**resolved** form, and normalisation has already turned the fragment into a
listed word by then. A stammer, a mimetic, or a shred of a contracted phrase
becomes a ledger row for a word nobody read.

| line | what comes out | the false row |
| --- | --- | --- |
| そんならあてぃしも… | そん + な + ら | 村/そん (rank 1,117) |
| あらあら……私？ | あらあら | 粗々 |
| くるんと、ノアは… | くるん + と | 包む/くるむ |
| びくん！ | びくん | 微醺/びくん (rank 9,695) |
| くすん……。 | くすん | くすむ |
| みし、みし、みし、と | みし | みせる |
| ざけんなぶっ殺すぞ | ざけ + んな | さけ |
| きいぃ！ / はわ、わわわ | きい / はわ | 聞く / 這う |
| 魔女……どもめ……！ | もめ (of どもめ) | 揉める |
| あんま時間もねーしな | あんま (of あんまり) | 按摩 (massage) |
| かたや〜、かたや〜 | かた + や | 方/かた |

**あてぃし is the same mechanism seen through a character's voice.** Sudachi's
Mode C shreds the childish pronoun (a drawn-out あたし) and recompose cannot
rescue it, because no listed headword combines the parts — so which real word
the shred lands on depends on the line: あて/当て + ぃ + し in 「あー、あてぃし？」,
あて/当てる in 「あてぃしは俯瞰することができた」, and しだ gated by the master's
羊歯 in 「あーあてぃしだよー？」. あん out of 「んで、あんさぁ」 lands on 案.
Blacklisting the pronoun spellings cannot reach these: 当て and 羊歯 are real
master headwords, asserted falsely.

**Two fences that look like they should catch the family both miss.** The noise
gate sees the resolved form; and the two-mora rank guard does not reach びくん
(three morae), nor 微醺 (rank 9,695), nor 村/そん (rank 1,117). かたや fails it
for the third reason — the match is to a real *common* headword rather than a
coincidental rare one.

**The class is split by which rung built it, and only one half is reachable.**

- **The reading fallback** — そん → 村, びくん → 微醺, きい → 聞く, どど → 度々.
  Nothing the text wrote was listed, so all that was left was the sound.
- **Sudachi's own normalisation, accepted at *Exact match*** — はわ → 這う,
  もめ → 揉める, みし → みせる, くすん → くすむ, くるん → 包む. The surface is a
  stem of a real verb and the pair lists, so the top rung fires and no fence
  below it is ever reached. The line is a mimetic or a stammer, and nothing in
  the token says so.
- **A genuine homograph** — あんま (of あんまり) is 按摩, あらあら is 粗々, かた
  (of かたや) is 方. The surface *is* the listed word; the sentence meant
  something else. Not separable by any rule.

**What was fixed is the part with a class signal on it**: a two-mora 感動詞 may
not take a kanji headword at the reading fallback, whatever its rank — an
interjection is never とき or はず, which is what the rarity fence exists to
protect. 145 tokens of crying (はは → 母 47, ひっ → 引っ 70, ぐう → 隅, ひい → 一,
あん → 案, くう → 九, おら → 俺) against one real word, くそ.

**Three widenings were measured and rejected**, and they are why the rest of
this group has no fix — see *What has been tried and measured wrong*. The short
version: every lever that reaches the remaining cases is the spelling-class
lever, and pulling it is the denominator decision rather than a parse fix.

### What is already fixed, and why the rule sharpens on its own

A kana term of at most three morae that **no dictionary lists** — asked in both
alphabets — is refused a ledger row entirely. 130 rows and 338 encounters of
sound effect and hook shrapnel: ぎい, ぐっ, ぎっ, ちゅぷ, くちゅ, ちゅる, ががが,
ズチュ, グチョ.

**It keys on the dictionaries, so it sharpens as they are added.** Installing
擬音語・擬態語辞典 (講談社, 1,967 headwords, 804 in nothing else here) took 15
terms straight back out of the noise bucket — きっ, にっ, がばり, ぷん, きゅっ,
ぼっ, くっ, ぷい, ざらり, そっ, ばっ, ぐっ, ぎっ, だっ, のっ are mimetics with
entries, and the rule was over-broad on them for exactly as long as nothing
listed them. ぎい, ちゅぷ, ズチュ and ががが stayed refused.

**Three morae is where the population turns.** Below it a kana string nothing
lists is almost always noise; at four and five it is the work's own vocabulary —
ダイイング, トレデキム, ジンザイ, ハルウリ, ヒトカリ are what one VN is *about*
and no dictionary will ever list them. **The alphabet is asked both ways before
the string is condemned**, which is what keeps ウチ (107), コイツ, ガッコ, ソレ,
ソッチ, ミライ and シケイ: a katakana spelling is a decision about how to write a
word, not evidence that there is no word.

### Waiting on a decision: one mora of kana is never a word

The loud half of the bucket is loud **because a dictionary lists it** — ちゅ ×207
is in 明鏡, ぢ ×52 in Jitendex, both one mora, both heading the work's triage
queue. The rule above cannot reach them by construction.

The rule that would is the identity ladder's own argument (`mora_of_kana`,
`headword_for_reading`) moved to the wordhood gate: **one mora of kana is never a
word of its own**, whatever the dictionaries say, because Japanese has a word for
every single kana and the match is found every time. Measured over the ledger:
**46 rows and 1,378 encounters** would go — ちゅ, ぢ, ひ ×358, う ×213, く ×153,
あ ×140, ふ, ぎ, ちょ — against **37 rows and 149,058 encounters that must
survive**, which are the particles (を は が に) and the affixes (さ お ご め).
Fencing on those two parts of speech is what separates them, so `is_noise` would
need the token's POS, which it does not take today.

Not built for two reasons, both worth a deliberate answer rather than a default:
it **overrules a dictionary**, which no other rule here does; and to stay
coherent it would have to drive the reader's paint as well as the ledger, or ひ
and あ would sit tinted `new` on every line forever instead of quietly `known`.
Blacklisting is the only lever that reaches them otherwise.

## 2. The denominator: a kanji identity for a kana line, and a kana line with no kanji identity

**~220 tokens, and a decision rather than a bug** — the same one asked from two
directions. See *Not parse defects — questions about the denominator*.

**The fold fires** — とうもろこし keyed 玉蜀黍, いびき 鼾, ご祝儀 御祝儀,
のしあがる 伸し上がる, 一人暮らし 独り暮らし (~60). The pair *is* listed, so the
exact-match rung fires and the "would add kanji the text did not use" guard —
which sits at the bottom of the ladder — is never reached. Refusing it would take
these words off the master scale, since いびき is not a Sankoku headword and 鼾
is. It is the same mechanism that makes いう and 言う one row.

**The fold has nothing to land on** — 158 tokens over 76 spellings the master has
no headword for at all: 蒼く, 碧い, 昏い, 視える, 還す, 喪っ, 忌々し, 廻天, 兄妹,
箱舟, 棄損, 誤魔化し. Sankoku does not list 蒼い or 視える under any reading, so
there is nothing to offer in place of the swap. Keeping the surface takes the
word off the scale; the swap keeps it on under a kanji the reader did not see.
The rest of that class is fixed — see the two swap entries under *Fixed*.

The mechanical test for the whole category is pure code: *neither the headword
nor its reading occurs in the line the token came from*.

## 3. A listed expression the join will not build

**~75 tokens, and a different fence each time** — which is why one group and
several fixes. The cost is a **missing word**, a lookup rather than a false
assertion.

| left as parts | times | refused by |
| --- | --- | --- |
| 満足 + げ (悲しげ, 不安げ, 悔しげ, 苦しげ, 憂いげ) | 68 | no segmentation dictionary lists the compound; the joined 得意げ and 意味ありげ work |
| と + は + いえ | 25 | two fences at once, and both were measured — see below |
| お + 経 | 3 | the length floor — お経 is two characters and only one is kanji |
| きわまり + ない | 3 | `reading_join_admitted` wants an all-`動詞` run or a kanji in the head, and an all-kana きわまり is neither |
| お + 花摘み, お + 伺いを立て | 2 | both join paths require the run to *begin* on a content word, so a leading 接頭辞 is never admitted |

**とはいえ needs two fences moved and neither may move.** 明鏡 and 小学館 both
list it, and it still never reaches `segments`: their entry carries no reading,
and `standard_entries` filters `reading != ''` in SQL. Put it back and the run
is still refused, because it opens on a particle and a standard dictionary may
not license that. Each change was measured on its own and both are under *What
has been tried and measured wrong* — 1,997 tokens of grammar for the first, から目
destroying 目を離す for the second. What is left is a named list of strings the
segmentation authority should hold that no dictionary supplies a reading for,
for one string at 25 sightings.

**げ and お are productive, so no dictionary lookup will ever finish them.** お
attaches to any 動作名詞 (お伺い, お願い, お答え) — the join needs to try the run
*without* a leading honorific and re-attach it. げ is the same question from the
segmentation side that 連帯感 is from the denominator side.

**きわまり drags a second defect behind it**, and it is wider than the join. The
wordhood gate passes on the *resolved* spelling — `In master dictionary: 極まり` —
while the identity ladder refuses that pair, because Sankoku's 極まり reads きまり
and only Jitendex lists 極まり/きわまり. So the headword stays as written and
`define` looks up the literal string きわまり, which is no dictionary's headword:
the popup says `Not in any dictionary`.

**くぼみ is that second defect with no join in it.** Sankoku lists 窪む and 窪 but
not the noun 窪み, so the ladder falls to *would add kanji the text did not use*
and the headword stays くぼみ — the right parse, and the gate passes it as `In a
standard dictionary: 窪み`. `define` then looks up `term = 'くぼみ'` exactly and
finds nothing, while 明鏡, 小学館 and Jitendex all hold the word as
`term=窪み/凹み, reading=くぼみ`. **A term bank keys on the kanji spelling even
where the dictionary itself writes the word in kana**: 小学館's entry prints its
headword as 「くぼみ」 — that is the `data-headword` span, with 窪み and 凹み filed
under 参考表記 — and it is still indexed as 窪み. There is no kana row to find and
no dictionary can be added that would supply one. The fix is in `define` rather
than the parse: on an all-kana term with no term match, retry on `reading`, which
`idx_dictionary_entries_reading` already indexes — handed the tokenizer's
candidate list (`窪み / くぼみ`) rather than the raw kana, since a bare reading
pulls 公園, 講演 and 後援 at once.

**確かに and たまえ are out of this group**, both under *Fixed*, and both had a
stated cause that was wrong. 確かに was refused for holding a bound stem while
本当に joined, and the whole difference was Sudachi calling one に a case
particle and the other the copula's 連用形. たまえ was blamed on
`with_standard`'s empty-reading skip, which is dead code; the real cause is that
Mode C keeps 待ちたまえ whole **only at the end of the input**, so it is a
boundary defect and belongs to group 5's mechanism, not this one.

## 4. Reading choice, where the spelling is right and the master lists several

**~30 tokens over eight sightings, and one structural cause.** The exact-match
rung takes Sudachi's pair whenever the master lists it, and nothing weighs the
alternatives; `preferred_readings` only covers readings the language has moved
off, so a live-but-wrong reading is never reached.

| line | read as | should be | times |
| --- | --- | --- | --- |
| 一日手伝うだけ | ついたち (and いちじつ) | いちにち | 15 |
| 旧教徒の集る場所 | たかる | あつまる | 8 |
| 今月の２８日までに | ２８/によう + 日/か | a date | 4 |
| 砂と塵が舞う | ごみ | ちり | 3 |
| 砂粒のように小さく | さりゅう | すなつぶ | 1 |
| ……何時なの | なんどき | なんじ | 1 |
| くそう、やはりダメか | 臭い/くさい | the interjection | 1 |

**Three of them are a reading only a `reference` dictionary knows.** Jitendex has
砂粒/すなつぶ and 集る/あつまる; only the master may name an identity, so nothing
overrules Sudachi. 何時 is worse than a close call — 何時まで joins into a
`助詞`-tagged compound read いつまで with no 何時 token at all, a category error;
the damage is confined to the literal spelling, since 217 of 218 何時 rows are
kana いつ normalised onto it, which is the rule working.

**Frequency would not have saved 塵**: BCCWJ and Jiten both rank 塵/ごみ far above
塵/ちり (2,661 vs 87,036 in Jiten), because the corpus writes ごみ in kanji and
modern fiction writes it ゴミ. Sankoku lists ちり first and twice. Worth
considering for the whole group: **when a spelling has several listed readings
and the choice came from Sudachi alone rather than from a preference or a rank,
lead with the master's order.** For 塵 the consequence is only ordering in the
popup — `define::definitions` filters senses to the reading the tokenizer chose.

**で and に read as the copula だ** belongs here by mechanism and nowhere else by
size: 4,336 tokens (で 1,257, に 3,079). **Most of it is not an error** — 綺麗に,
見事に, マジで, 必死で are na-adjectives whose adverbial *is* the copula's form,
and 〜ので (259 of the で cases) is the copula too. The wrong ones are で after a
plain noun — こと (55), 物 (49), 話, 犯人, 瞳. It is Sudachi's analysis rather
than the ladder's, and it costs nothing in the ledger since だ is grammar judged
long ago; the cost is the popup opening on the copula when a particle is tapped.

## 5. A boundary inside a word, or a word nothing lists

**~8 tokens, and the only class no downstream rule can reach** — recomposition
merges *whole* tokens, so a cut that falls inside a word is permanent.
`CUT_BEFORE_AND_AFTER` is a named list for that reason. **Measured and small**:
see *Boundaries that fall inside a word*, which found 4 real cases over 503,430
adjacent pairs.

- **チョロい** (4) — one missing SudachiDict entry, not a rule. ヤバい, エロい and
  ダサい are all listed in katakana and come out whole; ちょろい in hiragana does
  too. Only the katakana チョロい is absent, and チョロ alone is listed as an
  adverb, so the split wins — but **only when the sentence inflects it**:
  「だいぶチョロい気がします」 is whole, 「全然チョロく」 and 「どこがチョロいんだ」
  come apart, the latter putting the ん of んだ inside 忌む/イム.
- **ロボットがふよふよと** (2) — `ロボット` + **`がふ`** + `よ` + `ふ` + `よ` +
  `と`. The subject particle is swallowed into a nonsense 副詞 and the mimetic
  shredded, giving two false rows: がふ as an adverb and よ as the adjective よい.
  **No dictionary lists ふよふよ and none will** — mimetics are a productive
  system: 擬音語・擬態語辞典 has 1,967 headwords, surasura 1,422, and they overlap
  on 939, two independent attempts agreeing on 38% of the space. Admitting them
  by *shape* was measured and is no better — the ABAB template admits 9 corpus
  terms nothing lists (21 encounters), half of them カカカカ, イイイイ, どどどど,
  ぐぐぐぐ, screams and keyboard mashing. So the defect is the swallowed が, not
  the missing entry. **The rule that would fix it keys on the particle**: が, を
  and へ essentially never begin a Japanese word, so an unlisted token starting
  at a boundary and beginning with one has swallowed it. Fires about four times
  over the read corpus (ががががが, a scream), too little evidence to build on;
  10 sightings of ふよふよ in one script is the case for revisiting.
- **いやがおうにも** (1) — 否が応にも spelt in kana comes out `いや`/否 + `が` +
  `お` + `うに` + `も`, and うに becomes a ledger row. Same shape as なんて and
  また: a set phrase the rewrite pass should handle before Sudachi sees it.
- **天球儀** (1) — a compound no segmentation authority holds whole, split into
  two parts that are each listed.
- **最低減 (4, 白昼夢の青写真) — the script spells 最低限 with 減 for 限.**
  Sudachi splits it 最 + 低減, both master words, and the reading さいていげん
  names 最低限 alone — but a sounded join may not drop a kanji the text wrote,
  and 限 is not on the page. Keying 最低限 would assert a spelling nobody read,
  the same refusal that keeps 検死 off 検屍, so the split is the design.
  最低限 elsewhere in the same script is whole.

## Two the kanji rules cannot reach at all

Not a mechanism of their own — two lemmas share a surface, no kanji is dropped,
and nothing weighs them. Kept here because they look like the swap and are not:
なれる keyed on 慣れる where the line meant なる, 行って on 行く where it meant
行う, and いい on 言う where it meant 良い — 9 of the corpus's 19 いい→言う tokens,
the rest being といいます and 言いがたい, which are right.

## 6. The word with no popup: what the non-word gate refuses

**23 terms, 45 encounters, over the whole read corpus.** A token nothing lists
gets no span, so the reader cannot tap it and the popup never opens — the one
defect class that is visible while reading rather than only in the ledger. A
sweep of all 41,408 undiscarded lines found 601 such terms; 93 contain kanji,
and these are the ones that are real words arriving at the gate in a form no
dictionary carries. The rest are transparent compounds (33), the work's own
coinages (11) and hook garbage (9), all of which are correctly refused.

Four causes, and they are the causes already named above, seen from the gate's
side rather than the ledger's:

- **A compound split, and the fragment is not a word** (5): 虫眼 + 鏡越し for
  虫眼鏡越し, 独我 for 独我論, 已然 for 已然形, 輸管 for 精輸管, 送音 for the
  script's own 挿送音. Group 5's mechanism; the fragment costs a lookup on a word
  the popup would otherwise define.
- **Causative -す taken as the lemma** (5): 書かす, 読ます, 開かす, 張らす,
  手伝わす. The surfaces are 書かさ, 読まさ, 開かさ, 張らし, 手伝わさ — ordinary
  causatives of 書く, 読む, 開く, 張る, 手伝う, keyed on a 五段 lemma the master
  does not hold. One rule reaches all five.
- **Potential and bare stems left as the headword** (8): やり直せる, 使いこなせる;
  探し, 正し, 撫で, 悼み, 振り返り, 起き上がり. The word is right and the form is
  not, so the ledger keys a row nobody would look up.
- **Orthography the master spells otherwise** (4): 隣り町 for 隣町, ウワサ話 for
  噂話, 毒付く for 毒突く, 貶する for 貶す. Group 1's mechanism, reaching the gate
  instead of the count.

**A fifth cause, sighted 2026-08-21 and not in the sweep above: a
normalisation that takes every kanji off the page.** SudachiDict reads 小心 as
the Chinese name シアオシン and normalises it to that katakana, and the identity
ladder's last rung takes it — `drops_kanji` fences every candidate above it, but
the fallback is guarded by `adds_kanji` alone, so it is the one place a swap may
drop the kanji the text wrote. Nothing lists シアオシン, so the gate refuses it
and 小心 gets no span — 者 beside it is fine, so the line offers a popup on the
suffix and none on the word. The context decides which SudachiDict
entry wins: 小心な男 is right, and 小心者, 小心者め and 小心 alone all take the
name. The master lists 小心/しょうしん, so the rung that would fix it keeps a
surface the master holds under *some* reading rather than a katakana form
nothing holds. 小心者 itself is Jitendex-only, so the join has no listed
compound to build either.

**A sixth, sighted 2026-08-21: a lemma nothing lists, where the surface is the
master's own headword.** SudachiDict has the verb 鷲掴む, which no reader's
dictionary carries, and reads 鷲掴み as its 連用形. The ladder's surface rung is
fenced to an uninflected token — a stem that happens to be listed is a different
word, 許せ against 許せる — so the surface is never offered, the candidate list
comes out empty, and the fallback keys 鷲掴む. The gate then refuses it: no
popup, and Sankoku's 鷲掴み/わしづかみ is never counted. **The preceding particle
decides it**: 鷲掴みにした, 鷲掴みにされた and 鷲掴みだ are all right, and
胸を鷲掴みにした is not, because を in front makes the verb path cheaper. The kana
spellings 鷲づかみ and わしづかみ key on 鷲掴み in every context, since SudachiDict
has no verb written that way. The rule that would fix it is narrow: **where the
lemma is in no dictionary at all and the surface is a master headword read the
way the token reads, the surface wins** — 許せ is refused by the same rule
because 許せる is listed.

**The 17 dictionary gaps found in the same sweep are not parse defects.** 殺し合い
(13), 居た堪れる (8), 先走り (5), 羽ばたき (5), 吐精, 心置き, 投げ矢, 滅び, 自罰,
血しぶき, 車通り, すり替え, 尿量, 幾筋, 祝い金, 赤丸, 遼々 — 51 encounters where
the parse is right and no loaded dictionary holds the term. Nothing in the
pipeline can fix those; a dictionary can.

## The name gate, on a term that is only ever a word

**一発屋, sighted 2026-08-21 in 「一発屋にならなかったわね」.** The identity is
right — Sankoku, 明鏡 and Jitendex all list 一発屋/いっぱつや — and the token is
dropped anyway: SudachiDict tags it 固有名詞, and every escape misses. No cast
list holds it, `NOT_A_NAME` does not, and `ordinary_headword` wants okurigana,
which bare kanji does not carry. The highlighter drops a proper noun before it
consults the ledger, so the word has no popup and no row.

**One term, not a rule.** 花屋, 本屋, 質屋, 八百屋, 居酒屋, 照れ屋, 殺し屋, 極道,
太鼓持ち, 三日坊主, 風来坊 and 土左衛門 all come through as words, so nothing
about the shape is at fault and this is a `NOT_A_NAME` judgement, the same as 眸
and 予定調和. It is the easier half of 仁王立ち's defect, listed under *Fixed*:
there the name-tagged morpheme really is a name and only the join is wrong,
where 一発屋 is never a name in a sentence.

## A join that eats the suffix in front of it

**美しさまで, sighted 2026-08-21 in 「――お前の美しさまでは、奪えなかった」.**
Sudachi is right — 美し (形容詞 語幹) + さ (接尾辞 名詞的) + まで — and the join
takes the last two, because Sankoku lists さまで, the archaic adverb of
「さまで気にしない」, as a kana headword. The suffix that makes 美しさ a noun is
pulled off the word it belongs to, so the line credits an adverb nobody wrote
and leaves 美し keyed on plain 美しい. `NEVER_JOIN` is the shape of the fix.

**Two different token pairs spell it, and both fire.** The other is おかげ + さま
in 「おかげさまで」, which arrives as a second defect stacked on the first:
Sudachi hands over おかげさま whole and the wordhood gate refuses it — no
segmentation authority lists the kana, 明鏡 has only 御蔭様 — so mode A splits it
into おかげ + さま and the join then builds さまで across the seam. Fixing the
join alone leaves おかげさま in two pieces.

**Three of the corpus's eight さまで lines join, and none of them is the
adverb**; the ledger row reads two encounters. 高さまで survives only because
Sudachi keeps 高さ as one token, so the collision is confined to the suffixes it
hands over separately. The rule worth weighing beyond the named string: a
接尾辞 binds to what precedes it, so it cannot open a join run.

---

# Not a parse defect — one surface for looking up, another for counting

**Noted, not designed.** The reader and the ledger are asked different
questions, and the tokenizer currently answers both with one output.

When reading, anything the eye stops on should be lookupable — a substring, a
guess, a run the parser refused to commit to. Nothing is asserted by a lookup,
so the cost of offering a wrong candidate is a glance. That argues for the
JL/Nazeka/Yomitan behaviour: deinflect at the cursor, offer every candidate the
dictionaries have, rank them, let the reader pick.

When ingesting, every token becomes a row, a count and a triage position. There
the cost of a wrong answer is a false assertion that outlives the session, which
is why the rules above refuse rather than guess.

Today the reader's spans come from the same tokens the ledger keys on, so a
refusal to commit is also a refusal to *offer* — 「ふよふよ」 has no span, and a
token kept as written can be tapped but only for the string the parser chose.
Splitting the two would let the parser get stricter without the reader losing
reach, which is the direction every rule in this file pushes.

The pieces are already separate in the schema: `highlight` produces offsets,
`define` answers a lookup, and `Wordhood` is a distinct set from the master. The
work would be giving the overlay a lookup path that does not go through the
ledger's key at all.

---

# Not parse defects — questions about the denominator

The vocabulary scale is the master dictionary alone (`COUNTS_AS_VOCAB` is
`in_master`), by design: adding a dictionary must change classification and
never the denominator. These three all reduce to whether that rule should bend,
and none of them is a wrong token.

- **A 連用形 noun whose verb the master lists.** 聞きかじり, 書き込み, 窪み —
  Sankoku has 聞き齧る, 書き込む, 窪む and not the nouns. All three are clickable
  and defined; all three are off the scale.
- **A productive suffix the master lists only as the bare stem.** 連帯感 (and
  劣等感, 疎外感); げ is the same question from the segmentation side.
- **A genuine gap.** 蠱毒 is in Jitendex and in no dictionary of the master's
  size, and no rule will recover it. The honest outcome may be a fourth state
  between `non-word` and a scale entry: known to exist, not on the count.

An okurigana or kana variant of a master headword — あばら家 against 荒ら家,
依代 against 依り代 — was in this list and is out of it: both are words with a
popup now. Putting them on the *scale* would mean loosening the guard that keeps
其れ and 此の out of the corpus, so it needs to key on the master listing the
reading unambiguously, not on the spelling.

---

# Measurement records

What was measured, when, and what it said — kept because a later change is
judged against these numbers. **They are not open items**: where a record names
a defect, its current state is in the two lists above.

## Every word the reader cannot open — 2026-08-19

`jp_core::highlight::analyze` over all 41,408 undiscarded lines, counting tokens
excluded as `non-word` — the same call the feed and `#tokenize` make, so this is
what the reader actually sees.

844 lines (2.0%) carry at least one unopenable word; 1,191 of 627,012 tokens
(0.2%), 601 distinct terms, no blacklisted term anywhere. 93 of the 601 contain
kanji and were read one by one against the line they appeared in: 23 parse
defects (group 6), 17 dictionary gaps, 33 transparent compounds — mostly the
suffixes …側, …内, …越し, …まみれ, …状, …刻み — 11 coinages, and 9 that are not
text. Seven of those nine (呻呻, 慄慄, 戦々, 掻掻, 攣攣, 痒痒, 痙痙) come from a
single hook-garbled line where every character is quadrupled.

The 508 kana-only terms were not read individually: the noise gate already holds
that population, and its head is mimetics and sex-scene sound effects.

## Boundaries that fall inside a word — 2026-08-16, and the class is small

チョロい is not one word but the one defect class no rule downstream can reach:
recomposition merges whole tokens, so a cut that falls *inside* a word is
permanent. That is why `CUT_BEFORE_AND_AFTER` is a named list. The obvious worry
is that a named list is hiding an unmeasured mass, so it was measured.

Over every adjacent token pair in the corpus — **503,430 of them** — take the
pair `A | B` and ask whether `A` plus some prefix of `B` is a word the
segmentation authority lists while `A` itself is not, in either alphabet. That
is the shape of a cut that came too early, and it is pure code with no judgement
in it.

**14 pairs, and 4 are real**: チョロ | いん, 虫眼 | 鏡越し, ぼっ | ちゃま,
針葉 | 樹林. The rest are laughter and moans the noise gate already refuses
(えへ | へへへ, ひえ | ええ, ちゅ | うう). A second pass shaped for 牛乳粥 —
`A` listed, `B` not, and both halves of a different cut listed — adds
言わ | んかっ and 知ら | んかっ, the dialect 〜んかった, and nothing else.

**What the probe cannot see, and it is the bigger half.** A cut inside a word
*no dictionary lists* is invisible to a dictionary oracle — 念動力, メインルーム,
ダイイング, トレデキム are the work's own vocabulary and nothing will ever list
them. Only reading lines as sentences finds those, and that audit put the whole
"split: nothing lists the whole" class at ~10 of 2,499 tokens. So: the part a
rule could fix is a handful, and the part that is left is not a boundary problem
but a wordhood one.

## The top of 白昼夢の青写真's script queue — audit of 50, four causes

Not one word: the fifty commonest unjudged terms in the work's script profile,
each checked against a line it appears in and the token the pipeline built.

| class | terms | occurrences |
| --- | --- | --- |
| parse errors | 14 (28%) | 3,865 (49%) |
| names leaked into vocabulary | 8 (16%) | 2,276 (29%) |
| useful vocabulary | 27 (54%) | 1,638 (21%) |

The inversion is the finding. By term the queue is about half junk; **weighted
by how often the words occur it is 78% junk**, because the errors cluster at
exactly the frequencies a triage session starts from.

**A name Sudachi has no entry for is split, and the fragment is counted.**
The 皆守 case, four more times, and it is the single largest cause here:

- 凪 ×2,385 — 世凪 as 世 + 凪
- 李/すもも ×682 — the character すもも normalised onto the fruit
- 麻 ×84 — 入麻 as 入 + 麻
- 鯱 ×70 — シャチ normalised onto the orca

**A name joined to grammar.** 凛と ×72: the line lists two people, 凛 and
オリヴィア, and 凛 + と was joined into the adverb 凛と. Recomposition refuses a
run containing a proper noun — 凛 was not tagged one.

**Fragments of a longer word.** 乳粥 ×47 out of 牛乳粥; 症 ×45 off 症状.

**Bad joins and readings.** この家 ×48, joined and read このや where the line is
この + 家. 玉蜀黍 ×39, keyed in kanji for a line that wrote とうもろこし — the
spelling fallback did not fire.

**Never vocabulary in any work.** ちゅ ×207, ぢ ×52, ちゅる ×29 — onomatopoeia
from sex scenes, and the ×207 shows how much of it there is. YOU ×51 and ME ×54
are Sudachi normalising kana onto ASCII. These want a wordhood rule, not an
entry each.

Two things worth keeping from how this was measured. **Five of the fifty had no
literal match in the script at all** — the ledger key is a spelling the text
never used — and that test is pure code: if neither the headword nor its reading
occurs in the line the token came from, something has been asserted that was not
read. It caught 玉蜀黍, オリーブ, 鯱, YOU and ME without judging anything.

And the per-work name list already being imported (`work_names`) covers 6 of the
14 errors and all 8 leaks: **6,141 of the 7,874 occurrences audited, 78%**. It
is not a polish step, it is most of the problem.

## The same script sampled at random — the tail is a different defect

The audit above took the fifty *commonest* unjudged terms and concluded the
errors were concentrated at the head. That conclusion did not follow: a
top-only sample cannot say anything about the tail. So: fifty more, drawn
uniformly at random from all 5,440 unjudged terms of the same work (counts 1
to 13), checked the same way.

| class | top 50 | random 50 |
| --- | --- | --- |
| parse errors | 28% | 6% |
| spelling errors | (counted as parse) | 8% |
| names leaked | 16% | **0%** |
| **junk** | **44%** | **14%** |
| useful vocabulary | 54% | 82% |

The head really is about three times worse, and now that is measured rather
than assumed. But the two ends fail in different ways, and each wants its own
fix.

**Names are a head-only defect, necessarily.** A cast member is repeated
hundreds of times, so a split name can only ever land at a high count — zero
appeared in the random draw. `work_names` is aimed exactly where the damage is.

**The tail's largest defect is spelling, not segmentation.** Four of the fifty
are keyed in kanji the line never used: 鼾 for いびき, 伸し上がる for のしあがる,
御祝儀 for ご祝儀, 独り暮らし against the text's 一人暮らし. That is the 玉蜀黍
case from the head audit, and this is where it lives. The surface-preserving
fallback — normalise no further than the spelling the reader saw — is either
not firing or not covering okurigana and prefix variants.

The three ordinary parse errors: 36 counted as a word (a number is not
vocabulary), 寝よう cut to the noun 寝, and 頂 broken out of 絶頂.

**Neither end needs a model.** The head is fixed by importing the cast, the
tail by making the fallback hold. And the mechanical test from the previous
entry — *neither the headword nor its reading occurs in the line the token came
from* — is precisely the definition of the spelling defect, so one check with
no judgement in it finds the whole category. Across 5,440 unjudged terms a 14%
tail rate is on the order of 760 wrong rows, which is worth a pass of its own.

## Where the parser stands — 2026-08-14, after the six fixes

Two fresh uniform samples, drawn by `examples/audit.rs` over the 31,655 lines
the reader actually read (`discarded = 0` — see below), and judged one at a
time against the line each came from.

**By token, weighted by occurrence — 59 of 60 right.** This is what the ledger
accumulates, and it is a flattering number: about three fifths of any such draw
is だ, た, は, を, に, て, と, か, ます, and those are never wrong. The one
error was で in 「どこで殺意を抱くか」 read as the copula だ rather than the
locative particle, which costs nothing because だ is grammar the reader has long
since judged.

**By type, one vote per distinct identity — 51 of 60 clean, 5 wrong, 4
arguable.** This is the number that matters, because a wrong type is a wrong
ledger row however rarely it occurs.

| what was wrong | sample |
| --- | --- |
| onomatopoeia counted as vocabulary | ぎぎぎい, ズブブ |
| a fragment of a word | こう (out of ちょこーっと), す (out of すべき), 送音 (out of 挿送音) |
| arguable — orthography or a split | やり難い, 母様 (off 御母様), ゲーム+オーバー, Closed → クローズド |

**Not comparable to the 14% at the top of this section**, which sampled the
*unjudged* terms of one work; this samples every distinct identity in the corpus
and so includes 部活, 趣味, 我慢 and every other word that was never in doubt.

The composition is the finding. **Segmentation and identity errors did not
appear in either draw** — the classes this file was built out of. What is left
is a wordhood question: noise that is not a word at all. `vocabulary` holds 341
distinct short all-kana non-master terms still `new`, across 3,443 encounters,
and that bucket is where the next pass belongs.

### The corpus is not what `lines` says it is

106 lines — 0.33% of them — carry **21% of every character in the table**. They
are half-width-katakana mojibake from one badly-hooked session of
素晴らしき日々 on 2026-07-21, 2–3k characters each.

All 106 are already `discarded = 1`, and ingest, the ledger and the daily counts
all filter that flag, so nothing downstream ever saw them. **Any new analysis
must filter it too**: before it did, a uniform token draw came back 32% rubble
and the name audit's three largest entries (キー ×2220, 泉 ×892, タン ×231) were
all of it.

## 160 lines read as sentences — 2026-08-15, and where the errors actually are

Uniform draw over the 33,949 lines read, **judged as sentences rather than as
tokens**: the line first, then every token checked against what the sentence
meant. 2,499 tokens.

Drawn from a `examples/tokens.rs` dump that was two dictionaries short of
production — it took the `standard` role from the command line rather than by
role, so 明鏡 and 小学館 were absent. Re-checked afterwards against a correct
dump: **147 of the 160 lines are identical**, and all 13 that differ are fixes
made since, plus 気を付ける joining as the standard dictionaries intend. The
classes and the rate below stand. This is the draw that found the join class, and the reason
it found it is the method — see *Where the value is not*.

| class | tokens | fixable how |
| --- | --- | --- |
| noise, one-mora shrapnel | ~25 | wordhood; two lines carry all of it |
| **spelling class** (deliberate) | ~25 | a denominator decision, not a fix |
| wrong identity, right span | 12 | ten of them the kanji swap above |
| **split: nothing lists the whole** | ~10 | not by joining — see below |
| **join refused: a listed conjunction** | 8 | the content-word fence |
| **join fired: a grammar point** | 7 | the clause-initial signal |
| names | 3 | 皆守って → 皆 + 守る, 佐奈実, hiragana のあ |
| particle filed under the copula だ | ~3 | Sudachi's analysis |
| numbers | 2 | ９０ read きゅうれい |

**~70 of 2,499 tokens, 2.8%**, excluding the spelling class; about a third of
lines carry at least one. The earlier uniform *token* draws put this at 1 in 60,
and both numbers are right — half of any token draw is だ, た, は, を, and those
are never wrong. A rate per token flatters; a rate per sentence is what the
reader meets.

**The splits divide on whether anything lists the whole**, and that decides
whether there is a fix at all:

- **A segmentation authority lists it** — でも and だが are fixed
  (`CLAUSE_INITIAL_ONLY`), and so are 確かに and たまえ; とはいえ, なんでも and
  お経 are still refused, each by a different fence. See group 3.
- **Only Jitendex lists it** — 抵抗感, 死ね, 氷漬け, 許容量, 白濁液. `reference`
  role, so it decides nothing about segmentation, and that is the design rather
  than a defect.
- **Nothing lists it** — 念動力, メインルーム, わがはい (吾輩 is listed, the kana
  is not), リチャード三世. No rule over dictionaries reaches these.

## The name batch — 2026-08-15, and what it moved

Eight fixes in one pass, measured against the 32,353 lines already read and
against 白昼夢の青写真's whole script. **474 lines of the read corpus changed,
and every change was reviewed** by diffing `examples/tokens.rs` before and
after — twice, once with the cast switched off (`TOKENS_NAMES=off`) so the
identity rules could be judged on their own.

### The cast list, finally read

`work_names` existed, was filled from VNDB, and nothing consulted it.
`SudachiTokenizer::with_names` is the whole of the fix, applied in four places:

- **the gate** keeps a cast name whole, so the C→B→A pass cannot take it apart;
- **`join_names`** puts back together what Sudachi already split — 世 + 凪,
  タンバ + レイン, ウィリアム + ・ + シェイクスピア;
- **`split_names`** takes apart what Sudachi glued to a name, fenced to a whole
  form no dictionary lists whose remainder is grammar (凛と comes apart, ウィルス
  and 出雲大社 do not);
- **`to_token`** tags it, and spells it as the text spelt it, so すもも stays
  すもも rather than becoming the fruit 李.

Against the script, where the cast actually lives: **92 terms disappeared
entirely (3,644 occurrences) and 42 shrank (5,230 more)** — 凪/なぎ from 2,385 to
6, 世/よ from 2,412 to 42, 凛 1,442 → 0, 李/すもも 682 → 0, ロブ 118 → 0,
凛と 72 → 0, 鯱 70 → 0.

All five works read have their cast now, and against the 32,353 lines already
read that is **3,552 lines, every changed token a name or a fragment of one**:
ナノカ ×461, ココ ×421, ミリア ×391, ノア ×324, メルル ×289, ゴクチョー ×220 —
the "about 1,500 tokens" read-stats' CLAUDE.md had listed as unfixable — and
皆守 as 皆 + 守 across 185 lines of 素晴らしき日々. `POST /api/vocab/rebuild`
pruned 109 ledger rows and carried 7 judgements across.

Three things reading that diff caught, none of them visible from one work:

- **A romanized alias is prose, not a name.** VNDB gives "Prison guard", "Old
  man", "Magical Girl Riruru"; splitting those on their spaces made guard, man,
  Old and Girl into people. Only a Japanese form is split now, and the tokenizer
  is handed nothing without Japanese in it.
- **Some cast names really are the word, past any threshold.** 看守 is 24,066th
  in fiction — rare enough that a name would be believable — and means a prison
  guard 230 times in a VN set in a prison. `jp-script names <work> drop` records
  that judgement under its own source.
- **An empty surface let a run spell a name it did not contain.** 皆守 came out
  read ．みなまもる off a stray 「…」, since the join concatenated that token's
  reading in. `join_run` already refused an empty surface; `join_names` does
  now too.

Three things the list needed before it could be trusted:

- **A frequency veto.** VNDB lists 母 as a character of this work, and the work
  writes it to mean a mother constantly. `NAME_VETO_RANK` is 5,000 on the
  reader-facing list; 母 is 872nd and nothing else in the cast is inside 9,000.
- **The halves of a full name.** VNDB gives ウィリアム・シェイクスピア and the
  script says シェイクスピア, so `work_names::all` splits on ・ for the
  tokenizer. One character is never a part — ピーチ・ザ・ビッチ would otherwise
  teach it that ザ is somebody.
- **Names VNDB does not have at all.** `jp-script names <work> add` writes them
  under their own source, so a refetch cannot drop them: ウィル, ロブ,
  テンブリッジ, ハーミア, タンバレイン, ハチマル, リープ, パピー.

### The other six

| defect | fix | corpus |
| --- | --- | --- |
| ordinary words dropped as names (眸, 王子, 城, 金, 鏡, 悪魔, 予定調和) | `NOT_A_NAME` | 36 occurrences, +74 more in the script |
| 深い read ぶかい, 箱 read ばこ | a ladder rung under the re-derivation: the master's own reading when the two differ only by the first mora's voicing | 44 |
| この家 read このや | `NEVER_JOIN` | 3, and 48 in the script |
| 下に出る built out of 下 + に + 出る | `NEVER_JOIN` | 0 read, 6 in the script |
| 宣戦布告 split by the gate | a Mode C form commoner than all its parts is kept | 3 |
| うるせー read わずらい | **tried and backed out** — see *What was refused*; fixed later by kana arithmetic | — |

### What was refused

Three rules that looked right and were measured wrong. Recorded so they are not
tried again.

- **Arbitrating a spelling by how much its reading resembles the surface.**
  Preferring whichever listed reading shares an onset with the kana surface
  picks うるさい for うるせー, and over the corpus it also turns コイツ into
  此奴/こやつ, きわまり into 極まり/きまり, まじか into 間近 and いーっぱい into
  一杯. A reading is too weak a signal to arbitrate a spelling on — what the
  うるさい family needed was the ending itself, as arithmetic.
- **Taking the master's single reading whenever Sudachi's is unlisted.** The
  first form of the 深い fix. It rewrote 所為/せい to しょい and 出入/でいり to
  しゅつにゅう — spellings the master merely reads some other way, which are
  different words rather than one word's compound form. Voicing is a mechanical
  relation and is the only one that licenses the swap.
- **Refusing to key a kana line on the master's kanji spelling** — see the
  spelling-class entry. It is a decision about the denominator, not a parse fix.

---

# Fixed

One line each; the argument that settled it is in the code, next to the rule.

- **断腸の思い, 机上の空論 dropped as names** — SudachiDict tags a handful of
  everyday expressions 固有名詞. `ordinary_headword`: mixed script separates them
  from the cast, since a Japanese name carries no okurigana. 63 occurrences of
  16 terms admitted, no name moved.
- **眸, 王子, 城, 金, 鏡, 悪魔, 予定調和 dropped as names** — the same defect on a
  term with no okurigana, where nothing structural separates it from 橘 or 葵.
  `NOT_A_NAME`, one reviewed judgement per string. 36 occurrences.
- **A work's cast split, counted, or called a name only some of the time** —
  世凪 as 世 + 凪, すもも as the fruit, ウィル a name sixteen times and vocabulary
  eleven more. `work_names` is read by the tokenizer now; see the batch record
  above.
- **スッとする** — the mimetic split and the と then taken by とする. The join
  offers the run folded to hiragana, last and only for an all-kana run opening on
  a content word. 62 occurrences recovered.
- **いいんだよって** — the join built よって out of the sentence-final particle
  and the quotative. `NEVER_JOIN`, which cost nothing: the real 「彼によって」
  arrives whole from Sudachi.
- **この家 read このや** and **下に出る built out of 下 + に + 出る** —
  `NEVER_JOIN`. 48 and 6 sightings in one script, none of them the listed phrase.
- **ひとりもいやしない** — 弥 off the いや of 居やしない. A two-mora hiragana
  surface may not take a kanji identity the reader-facing list ranks worse than
  15,000. 169 occurrences, 90% of them wrong.
- **ムワムワ had no span at all** — the no-row wordhood path asked the master
  where a row is asked the lenient gate. `Highlighter` carries a `Wordhood` set
  now, so whether a word can be tapped no longer races the ingest watermark.
  This was the general clickability defect: 景気づけ, 花摘み, 砂粒, 連帯感, 蠱毒,
  依代, あばら家, 聞きかじり, 書き込み, 窪み all got their span back with it.
- **深い read ぶかい, 箱 read ばこ** — Sudachi reads them standalone with the
  compound's voicing, and the ladder's re-derivation asked the same wrong oracle
  twice. A rung under it takes the master's own reading where the two differ by
  nothing but the first mora's voicing. 44 occurrences.
- **宣戦布告 broken by the gate** — Mode C had it whole. A form the reader-facing
  list ranks above every part the split would produce is kept. Fires three times
  over the corpus, all one word.
- **36 counted as a word, 寝よう cut to 寝, 頂 broken out of 絶頂** — the three
  ordinary parse errors from the random-sample audit, all gone by 2026-08-15.
- **中には over 40 lines that say "inside", ときに over 32 of 35, 手を入れる
  over 8 of 9** — 「俺の袋の中には五円が入っていた」, 「小さいときに登れた」,
  「ポケットに手を入れ」. `NEVER_JOIN`, all three checked against every line
  they fired on. 中には had been cited as the reason the join admits three-part
  runs at all, and that reading of it had never been checked against a line:
  the expression meaning "some among them" occurs nowhere in the corpus.
- **うっさい read 煩い/わずらい** — the same word as うるせー said the other way,
  and no ending to un-contract: うっさい swallows うるさい's る into a small っ.
  The reading is read off the master's own list for the spelling Sudachi already
  chose, since nothing in the surface says which mora the っ stands for. 3
  tokens, and nothing else in the corpus moves.
- **何度 split into 何 + 度** — 104 sightings, all whole now, kept by the gate on
  明鏡's listing. Fixed by the standard role rather than by a rule of its own.
- **そういう built over そういえば 70 times** — 「そういえばあそこで、泣いてた」
  is "speaking of which", which Sankoku carries as そう言えば in its own right,
  not the conditional of そういう. `NEVER_BEFORE_A_CONDITIONAL`, checked against
  the ば after the run. Refused rather than rebuilt: no path reaches そう言えば,
  and widening the reading join to reach it was measured and is under *What has
  been tried and measured wrong*. What is left is そう + 言う + ば.
- **登れない read 上る where 登れば read 登る** — the swap's answer was fenced to
  a candidate whose *pair* the master lists, and Sudachi reads 登れ in 登れない
  off the potential 登れる, giving のぼれる, which is nobody's pair. The fence is
  the spelling now, since a swap wins on the spelling alone one rung further
  down, and the answer is looked up under the swapped spelling's own reading as
  well as the token's. 9 tokens, and nothing else in the corpus moves.
- **確かに refused while 本当に joined** — both are a word plus に and both
  Sankoku headwords, and the whole difference was Sudachi's tagging: 本当's に is
  a case particle, 確か's the copula's 連用形, so the no-inflected-part rule saw a
  stem in one and not the other. A 形状詞 followed by that 連用形 is its
  adverbial, not an inflection inside the run. 46 tokens — 確かに 16, 自然に 14,
  新たに, 滅多に, みだりに, 伊達に, 僅かに — and the result still has to be a
  listed headword, so 綺麗に and 見事に stay two words. ように is the shape the
  rule exists for and every dictionary here lists it, so `NEVER_JOIN` is the
  only thing that reaches it: 651 sightings, not one of them a word.
- **たまえ cut into た + まえ, with まえ keyed on 前** — blamed on
  `with_standard`'s empty-reading skip, which is dead code. The real cause is
  the lattice: Mode C keeps 待ちたまえ whole **only at the end of the input**, and
  「待ちたまえ！」 comes apart. Nothing downstream rejoins it either, since a run
  opening on the auxiliary た is a function word a standard dictionary may not
  license. `CUT_BEFORE_AND_AFTER`, 20 tokens, at the cost of one 待ち keyed on
  the noun rather than 待つ.
- **はは read 母, ひっ read 引っ** — a two-mora 感動詞 given a kanji headword by
  sound alone at the reading fallback. The rarity fence there cannot reach a
  word this common and is not meant to: it exists because とき is 時 and はず is
  筈 at the same length, and an interjection is never one of those, so at two
  morae its class decides on its own. 162 tokens over 8 spellings — はは 47,
  ひっ 70, ぐう 隅, ひい 一, あん 案, くう 九, おら 俺 — against one real word,
  くそ. Interjections only and two morae only; both wider forms were measured
  and are under *What has been tried and measured wrong*.
- **ものの built over もの + の 9 times of 26, ちゃんと over a name's ちゃん 76
  times of 174** — 「巨大なものの前で」, 「書かれたもののようで」, 「ヒロちゃんと
  友だちになりたい」. Both are decided by a neighbour, and neither by the one the
  list had guessed: what precedes ものの is た on both readings, and what follows
  it is a noun for the genitive and a clause for the concessive
  (`NEVER_BEFORE_A_NOUN`); ちゃん is the honorific exactly where a cast name
  precedes it (`NEVER_AFTER_A_NAME`). 85 tokens over 82 lines, every ちゃんと one
  of them a character being addressed by name.
- **ないと built over a quotative と 120 times of 379** — 「出ないと思う」,
  「信じられないという」. `NEVER_BEFORE_QUOTING`, checked against the token
  *after* the run, which `join_run` now receives. Named rather than a rule about
  と: さらりと言った and ぴしゃりと言った are the same shape and there the と
  belongs to the adverb. 「逃げないといけない」 is untouched.
- **そこで built over a place 48 times of 55** — 「俺と羽咲はそこで別れた」.
  The one conjunction `CLAUSE_INITIAL_ONLY` cannot take, since both its readings
  open a clause. `NEVER_JOIN`, which loses ~7 real ones and leaves them そこ +
  で: two words the reader knows, against a conjunction asserted over a place.
- **検死 keyed on 検屍, 上手く on 旨い, 綺麗 on 奇麗** — Sudachi's normalisation
  swapping one kanji for another, which changes *which word* is claimed rather
  than how fully it is spelt. A candidate may not drop a kanji the surface
  wrote, where the surface is a master headword as written — so the refusal
  costs nothing and the reader keeps their own spelling. 406 tokens over 116
  spellings, every one now keyed on itself. The mirror of the fallback's "would
  add kanji the text did not use", moved above *Exact match* because these land
  on pairs the master lists.
- **上手く keyed on 旨い, 抑え on 押さえる, 遭っ on 会う, 視 on 見る** — the
  inflected half of the swap, where the surface is a stem and so cannot be kept
  as written. The master's other spelling of the same reading answers it, and
  over the corpus the reading names exactly one such spelling every time but
  once (あかり is 灯 and 灯火 as well, and 灯り is neither). Offered only against
  a candidate the master lists, or it becomes a rung of its own ahead of the
  surface and rewrites あばら家 into 荒ら家. 429 tokens over 154 spellings.
  It costs five lines their join: 気を遣った and 手を挙げた were built out of
  気を使う and 手を上げる, and 気を遣う is Jitendex's alone, so the run no longer
  spells anything the segmentation authority lists and comes apart into 気 + を +
  遣う — three right tokens instead of one expression under a kanji nobody wrote.
- **ところで, すると, それで built everywhere; でも and だが built nowhere** —
  one defect from two sides, and position is the whole of the fix.
  `CLAUSE_INITIAL_ONLY` names five strings that are a word where they open a
  clause and two words anywhere else. 1,807 token changes over 963 lines: でも
  built 713 times and だが 121 where the sentence opened on them, ところで
  refused 65 times, すると 56 and それで 19 where it did not. Both general rules
  were measured and rejected first — refusing every mid-clause expression takes
  本当に and ために, and admitting every clause-initial two-kana master headword
  fires 1,975 times over 76 strings including 何が, では and して.
- **ウチ, アレ, コイツ keyed on themselves** — the master lists only the
  hiragana, so a katakana line opened a second row beside うち, あれ and こいつ.
  The fold is the last candidate on the identity ladder, so it can only win
  where nothing the text wrote is listed: スマホ and シャワー are katakana
  headwords, ザル and マジ match at the alphabet rung, and ハエ folds to はえ,
  which is nothing. 187 tokens, 25 spellings — including two identity fixes,
  アイツ off 彼奴/きゃつ and アンタ off 貴方/あなた. **It asks the cast list
  itself**: the name gate vetoes a cast name common enough to be a word and it
  asks the *identity*, so folding first put ココ's 421 sightings on the pronoun.
- **うるせー read 煩い/わずらい** — 〜あい and 〜おい contract to 〜えー, and
  Sudachi reads すげえ, くせえ and あぶねー right; 煩い alone carries a second
  reading. Kana arithmetic offering only a reading the master already lists for
  that spelling, and only onto a kanji spelling, since a kana headword matches
  on the headword alone and へえ would have taken はい. 17 tokens over three
  spellings of the held vowel — ー, え and the commonest, small ぇ.
- **なんてひどい → 何 + 手酷い, またいちから → ま + たいち + から, 牛乳粥 → 牛 +
  乳粥** — the boundary family, and the one class no rule over the finished
  tokens could reach: recomposition merges adjacent tokens and never moves a
  boundary. `CUT_BEFORE_AND_AFTER` is a named list of strings handed to Sudachi
  on their own, applied **only where the analysis shows the boundary actually
  came out wrong** — 14 lines over the corpus, against 59 when the cut was
  unconditional. See the entry below for why it is a list and not a rule.
- **ご愁傷さま, 大アリ, 物足り+なかっ split off their 接頭辞** — Sudachi hands
  these over in pieces (ご + 愁傷 + さま, 大 + アリ, 物 + 足り), and the reading
  join refused a run whose head is a 接頭辞. `reading_join_admitted` takes one
  now, behind the same fences as a 接尾辞: the combined reading names exactly
  one headword, and the answer keeps every kanji the text wrote — which is why
  最低減 still stays 最 + 低減 rather than becoming 最低限.
- **蛙 read かわず** — Sudachi hands back カワズ, and the identity ladder takes
  it at *Exact match*, since the master lists 蛙/かわず as a real entry. The one
  rung that overrules Sudachi on a reading is the preference map, and it
  declines: Jitendex scores かえる 200 and かわず 97, so the gap of 103 is under
  `POPULARITY_TIER`'s 150 and かわず lands in `acceptable`. With every reading
  acceptable the term never enters the map at all. The tier cannot simply be
  lowered — 街/まち and 身体/からだ score the same 99 against a 200 and are
  plainly the living readings. Frequency would settle it (BCCWJ 7,998 against
  15,356, Jiten 16,777 against 99,390), but rank is only a tie-break *among*
  acceptable readings and never shrinks the set.
- **仁王立ち → 仁王 + 立ち** — SudachiDict tags 仁王 `固有名詞`, being the temple
  guardians, and `join_run` refuses any run holding a proper noun before it
  looks at the spelling. The parts spell 仁王立ち exactly and all four
  dictionaries list it, so the strong signal was there and never asked.
  `names_someone`'s escapes all miss: no cast list has it, `NOT_A_NAME` does
  not, and `ordinary_headword` wants okurigana — 仁王 is bare kanji, structurally
  the same as 橘 or 葵. `NOT_A_NAME` would take it, but the class is wider than
  the term: 仁王 alone really is the statues, and what is wrong is only that a
  name-tagged morpheme cannot sit inside a word. The rule to weigh is letting an
  exact master-headword spelling overrule the name tag, which reaches the whole
  corpus.
- **いたって read 至って** — 「子供がいたって、…」 is いる + concessive たって
  ("even if there are children"), and Sudachi returns the adverb 至って as one
  token in Mode C, B and A alike, tagged `副詞`, `oov=false`. No stage of ours
  chose this: the gate keeps it because the master lists いたって, and the
  identity ladder takes it at *Exact match*. Nothing over the finished tokens
  can reach it, because the split い + たって is never offered.
  Both parses are real Japanese — 子供が至って元気だ is the adverb — so only
  context separates them. The one local signal is the comma: an adverb directly
  followed by 、 with nothing to modify is the concessive reading. That is a
  rule about what follows the token, which no existing stage expresses, and it
  would have to be narrow enough not to touch 至って before a genuine pause.
- **この前渡辺と → この + 前渡 + 辺** — 「この前渡辺といった居酒屋」 is この前
  ("the other day") plus the surname 渡辺, and Sudachi puts the boundary one
  character early: Mode C returns 前渡 + 辺. Both pieces then pass every later
  stage cleanly — the master lists 前渡し, so the gate keeps 前渡 and the
  identity ladder takes it at *Exact match* as まえわたし, and 辺 is a headword
  of its own. Nothing over the finished tokens can reach it; the boundary family
  again, so `CUT_BEFORE_AND_AFTER` is the only lever, and the string to hand
  Sudachi is この前. The cast list would also settle it — 渡辺 kept whole leaves
  この前 with nowhere else to go — but the surname belongs to no work here, and
  a per-work list is the wrong home for one of the commonest surnames in
  Japanese.
