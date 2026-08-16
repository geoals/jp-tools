# Parse defects

Words noticed misparsed while reading, worked through in batches. One entry per
word: what the pipeline does with it today, and the cause where it is known.

**21 open**, each checked against the live pipeline on 2026-08-16. Three
further questions are about the vocabulary denominator rather than the parse and
are kept apart from them; a fourth — whether the reader's lookup surface should
be the ledger's key at all — has its own section. What has been fixed is one
line each at the bottom.

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
sqlite3 ~/.local/share/jp-tools/knowledge.db ".backup /tmp/snap.db"
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
the length floor, not the content-word fence; たまえ by nothing in the tokenizer
at all).

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

**One mora of kana is never a word of its own, whatever a dictionary says.**
The identity ladder already carries this argument (`mora_of_kana`,
`headword_for_reading`) and the wordhood gate does not. Measured over the
ledger: **46 rows and 1,378 encounters** would go — ちゅ, ぢ, ひ ×358, う ×213,
く ×153, あ ×140, ふ, ぎ, ちょ — against **37 rows and 149,058 encounters that
must survive**, which are the particles (を は が に) and the affixes
(さ お ご め). Fencing on those two parts of speech is what separates them, so
`is_noise` would need the token's POS, which it does not take today.

Not built for two reasons, both worth a deliberate answer rather than a default:
it **overrules a dictionary**, which no other rule here does; and to stay
coherent it would have to drive the reader's paint as well as the ledger, or
ひ and あ would sit tinted `new` on every line forever instead of quietly
`known`. It is the only thing standing between the reader and a clean triage
queue — ちゅ ×207 and ぢ ×52 head 白昼夢の青写真's, and blacklisting is the only
lever that reaches them otherwise.

## Then, in value order

Done since this section was last written, all under *Fixed*: the katakana fold,
the colloquial adjective ending, the join's clause-initial list, `drops_kanji`,
ないと before a quoting verb, そこで / 中には / ときに / 手を入れる, the inflected
half of the kanji swap, and ちゃんと / ものの.

**The join list is finished.** ないか (415) and ないで (178) were measured with
the last two and are **right**: 「じゃないか」 is the negative question and
「言わないでください」 the negative te-form, which is what those entries are. Do
not re-measure them.

1. **The three denominator questions**, which are decisions rather than code.
   They change the headline number the whole system reports, and until one is
   made that number has an unstated policy inside it. **The kanji swap's residue
   is now one of them** — see its entry: 158 tokens are a spelling Sankoku has
   no headword for at all, and the choice is the swap or falling off the scale.
2. **The が swallowed into an unlisted mimetic** (the ふよふよ entry). The rule
   is clear — が, を and へ essentially never begin a Japanese word — but it
   fires twice over the read corpus today, which is not enough evidence.
   Revisit when more of 白昼夢の青写真 has been read; its script holds 10
   sightings of ふよふよ alone.
3. **Everything else in the open list, at one or two encounters each.** チョロい,
   一日, 塵, 砂粒, 何時, かたや, いやがおうにも, きわまり, お花摘み, 満足げ,
   天球儀, ２８日, くそう — the corpus dump has 1–4 of each. Worth doing as one
   batch on a day when the tools are already rebuilt, not one at a time.

## What has been tried and measured wrong

Do not build these; each cost a pass to disprove.

- **A general clause-initial join rule.** Admitting every clause-initial two-kana
  master headword fires 1,975 times over 76 strings — 何か, 何が, 何を, では,
  だと, して — because a clause opening on a pronoun and a particle is just a
  sentence starting. And refusing every mid-clause expression takes 本当に,
  ために and すぐに. Both directions are named lists.
- **A rule about と before a quoting verb.** さらりと言った and ぴしゃりと言った
  are joins ending in と before 言う and the と belongs to the adverb.
- **`with_standard`'s empty-reading skip as a hole.** It drops 14,064 kana
  headwords, but they are the reading-index rows those Yomitan builds carry, and
  `dictionaries::standard_entries` filters `reading != ''` in SQL first — so
  admitting all of them changes **zero** tokens.
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
| ないと before a quoting verb | 120 |
| ちゃんと after a name, ものの before a noun | 85 |
| 中には, ときに, 手を入れる | 84 |
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

The lookup-tax study (4,138 lookups over 26 days, never analysed) and the
denominator decision are still worth more than the tail of this list.

---

# Open

## チョロい → チョロ + いん, and the katakana i-adjective SudachiDict does not list

「どこがチョロいんだわたしの！　ビッチ！」 comes out チョロ (副詞) + いん, and
いん takes the identity 忌む/イム. The ん of んだ is inside the second token, so
this is the boundary family — recomposition cannot reach it.

The cause is one missing SudachiDict entry, not a rule. ヤバい, エロい and ダサい
are all listed in katakana and all come out whole with ん + だ after them;
ちょろい in hiragana comes out whole too. Only the katakana チョロい is absent,
and チョロ alone is listed as an adverb, so the split wins. The master lists
ちょろい, so the identity would resolve through the katakana fold once the
boundary is right.

**Only when the sentence inflects it.** 「だいぶチョロい気がします」 comes out
whole; 「全然チョロく」 and 「どこがチョロいんだ」 come apart, because the plain
form matches チョロ + い and any other ending sends the lattice through the
listed adverb. Four splits over the corpus, one of them the いん.

`CUT_BEFORE_AND_AFTER` is the lever; whether one string at four sightings earns
a list entry is the open question. **What settles it is that the class is
measured and small** — see *Boundaries that fall inside a word*.

## とはいえ, 確かに, たまえ — a listed expression the join still will not build

**たまえ is not the cheap hole it looked like.** `with_standard` skips an entry
whose reading is empty, and that is 14,064 kana headwords across 明鏡 and
小学館 — but almost all of them are the reading-index rows those builds carry
(あいすくりーむ, あいえっち, あい), not orthographic headwords. Admitting the
lot was measured: **zero tokens change over the corpus**, because
`dictionaries::standard_entries` filters `reading != ''` in SQL before the
tokenizer ever sees them. The skip in `with_standard` is dead code on the
production path, and たまえ is missing for a reason further upstream.

What is left of the under-firing side after the clause-initial list, and it is
three different fences rather than one. The trace names each:

| left as parts | times | refused by |
| --- | --- | --- |
| と + は + いえ | — | the conjugated-tail path needs a content-word head, and と is a particle |
| 確か + に | 16 | `Invalid expression: contains a bound stem` — に is the copula's 連用形 |
| た + まえ | 19 | never offered: 小学館's たまえ has no reading, so `with_standard` skips it |
| お + 経 | 3 | the length floor — お経 is two characters and only one is kanji |

たまえ is the one worth doing first, because it is not a judgement call: a
standard-dictionary entry with an empty reading is dropped from `segments`
entirely, and that is a silent hole in the segmentation authority rather than a
rule. 「待ちたまえ」 comes out 待ち + た + まえ, and まえ is keyed on 前.

## The kanji swap — the spelling the master does not have

**Both halves of the swap itself are fixed.** A surface that is a master
headword keeps its own spelling (`drops_kanji`); a surface that is a stem takes
the master's other spelling of the same reading — 上手く → 上手い, 抑え → 抑える,
遭っ → 遭う, 穢さ → 穢す, 登れ → 登る, 視 → 視る. 429 tokens.

What is left is **158 tokens over 76 spellings the master has no headword for at
all**, so there is nothing to offer in place of the swap: 蒼く, 碧い, 昏い, 視える,
還す, 喪っ, 忌々し, 廻天, 兄妹, 箱舟, 棄損, 誤魔化し. Sankoku does not list 蒼い
or 視える under any reading.

That makes the residue a **denominator question rather than a parse one**, and
the same one the spelling class below asks from the other direction: keeping the
surface takes the word off the master scale, and the swap keeps it on under a
kanji the reader did not see.

Two more that no kanji rule reaches, both from the 160-line sample: なれる keyed
on 慣れる where the line meant なる, and 行って on 行く where it meant 行う. Two
lemmas share a surface, no kanji is dropped, and nothing weighs them.

## くそう — read 臭い/くさい

「くそう、やはりダメか」 is the interjection. Sudachi normalises it onto 臭い and
the pair lists, so the ladder stops at *Exact match*. Same family as うるせー and
not reachable by the same arithmetic: くそう is くそ plus a drawn-out う, not a
contracted adjective ending.

## 天球儀 → 天球 + 儀

A compound no segmentation authority holds whole, split into two parts that are
each listed. One sighting.

何度 was the other half of this entry and is fixed: 104 whole over the corpus,
none split, kept by the gate on 明鏡's listing.

## ２８日 — read によう + か

`２８/28/によう` + `日/日/か`. The digits are read as a word and the counter
takes its bound reading, so a date comes out as two tokens neither of which is
a number. Numbers are already off the vocabulary scale, so the cost is the
popup and the trace rather than a ledger row.

## で and に read as the copula だ

「あの村で」, 「形で」, 「不意に」 — the case particle filed under だ. Three
sightings in a random draw of 20 lines, which makes it the most frequent single
error in the pipeline by rate.

**Most of it is not an error**, and that is why it is one entry rather than a
batch. 綺麗に, 見事に, マジで, 必死で are na-adjectives whose adverbial *is* the
copula's form, and 〜ので (247 of the 1,194 で cases) is the copula too. The
wrong ones are で after a plain noun — こと, 物, 話, 犯人, 瞳, 一人.

It is also Sudachi's analysis rather than the ladder's: the normalised form
arrives as だ and every candidate agrees. And it costs nothing in the ledger,
since だ is grammar judged long ago. The cost is the popup opening on the copula
when the reader taps a particle.

## ロボットがふよふよと — the particle is swallowed into an unlisted mimetic

`ロボット` + **`がふ`** + `よ` + `ふ` + `よ` + `と`. Sudachi has no ふよふよ, so
the cheapest path over 「ロボットがふよふよと」 takes the subject particle が into
a nonsense 副詞 and shreds the mimetic. Two false rows come out of it: がふ as an
adverb, and よ resolved to the adjective よい.

**No dictionary lists ふよふよ at all** — not Sankoku, not 明鏡, not Jitendex,
and not either of the two onomatopoeia dictionaries installed to look for it
(擬音語・擬態語辞典 and surasura, both removed again once they had answered) — so
even a perfect segmentation leaves it a `non-word`. What is lost is the が, and the two
assertions.

**And no dictionary will.** Mimetics are a productive system rather than a
closed list: 擬音語・擬態語辞典 has 1,967 headwords and surasura 1,422, and they
overlap on 939 — two independent attempts at the same space agree on 38% of it.
Admitting them by *shape* instead was measured and is no better: the ABAB
reduplication template admits 9 corpus terms nothing lists (21 encounters), and
half of them are カカカカ, イイイイ, どどどど and ぐぐぐぐ — screams and
keyboard mashing. The tail of this class is not recoverable by wordhood at all,
which is why the defect here is the swallowed が and not the missing entry.

Same family as the boundary defects under *Fixed*, and `CUT_BEFORE_AND_AFTER`
cannot take it: mimetics are coined freely and no named list will hold them. The rule that would is one keyed on
the *particle* rather than on the word — が, を and へ essentially never begin a
Japanese word, so an unlisted token that starts at a token boundary and begins
with one of them has swallowed it. Measured over the read corpus that fires
about four times (ががががが, a scream), which is too little evidence to build
on; 10 sightings of ふよふよ in one script is the case for revisiting it.

## 何時 — the clock reading なんじ is never produced, and the まで/でも forms vanish

Three spellings of one word, and the pipeline gets the literal one wrong every
way it can:

| line | what comes out |
| --- | --- |
| 「……何時なの」 | 何時 / **なんどき**, `名詞` |
| 学校は何時までかかるだろうか。 | 何時まで / **いつまで**, `助詞` — no 何時 token at all |
| いつ何時でも | いつ → 何時/いつ, then 何時でも / いつでも, `助詞` |

Both lines mean なんじ and no path produces it. なんどき is a real reading but
belongs to いつ何時, which is the one context here that *doesn't* get it. And the
join returns a compound of noun + particle tagged `助詞`, with a reading asserted
over the whole thing — a category error rather than a close call.

Low volume: 217 of this work's 218 何時 rows are kana いつ normalized onto the
master's spelling, which is the rule working. The damage is confined to the
literal spelling.

One thing to note when it is fixed, because it looks like a second defect and is
not: 何時/なんどき paints `known`, borrowed from the known 何時/いつ row by the
judged-under-another-reading rule. That rule is right; it was applied to a key
that should never have been built.

## 一日 — read ついたち where the line means いちにち

`preferred_readings` derives no entry that reaches this: 一日 has four master
readings and ついたち is among the *acceptable* ones, so the correction never
fires. It is right for a date and wrong for 「一日手伝うだけ」, and nothing in
the token says which.

## 塵 — read ごみ where the sense is ちり

Sudachi returns ゴミ, the master lists 塵/ごみ, and the ladder stops at *Exact
match* with a single candidate. No alternative is ever weighed.

The frequency tables would not have saved it — BCCWJ and Jiten both rank 塵/ごみ
far above 塵/ちり (2,661 vs 87,036 in Jiten), because the corpus writes ごみ in
kanji and modern fiction writes it ゴミ. Sankoku lists ちり first and twice.

**The consequence is ordering in the popup, not loss.** `define::definitions`
filters senses to the reading the tokenizer chose, so the first thing drawn is
ごみ; the other readings are still reachable through the expansion chips. Worth
considering: when a spelling has several listed readings and the choice came
from Sudachi alone rather than from a preference or a rank, lead with the
master's order.

## かたや — read as 方/かた

`かた` + `や`. The conjunction かたや is not listed, so the ladder takes the
two-mora kana run as the master headword 方/かた on an exact spelling-and-reading
match. The short-kana guard is the rule that should catch this and does not,
because the match is to a real common headword rather than a coincidental rare
one.

Cost is not the ledger row — it is that the popup opens on 方 and the reader has
to work out that the word was never there.

## いやがおうにも — 否が応にも, spelt in kana

`いや`/否 + `が` + `お` + `うに` + `も`. Produces two junk tokens; うに is a
ledger row. Same shape as なんて and また: a set phrase the rewrite pass should
handle before Sudachi sees it.

## 豪華きわまりない — the kana half of a word no dictionary spells in kana

`豪華` + `きわまり` + `ない`. Two defects stacked, and the second is the reason
the reader sees anything at all.

1. **The join never fires.** The word is 極まりない, which every installed
   dictionary lists, Sankoku included — but only under that spelling, so the
   spelling path finds nothing for きわまり+ない. The reading path would find it
   (`by_reading["きわまりない"]` is one headword), and it is not admitted:
   `reading_join_admitted` wants either an all-`動詞` run or a kanji somewhere
   in the head, and an all-kana `きわまり` is neither.
2. **The popup then says `Not in any dictionary`.** The wordhood gate passes on
   the *resolved* spelling — `In master dictionary: 極まり` — while the identity
   ladder refuses that pair, because Sankoku's 極まり reads きまり and only
   Jitendex lists 極まり/きわまり. So the headword is kept as written, and
   `define` looks up the literal string きわまり, which is no dictionary's
   headword.

The gate and `define` asking different questions is the general shape: a gate
match on a kanji spelling guarantees nothing about the popup, whenever the
ladder falls through to the kana surface. Fixing (1) removes this instance; the
mismatch itself is wider.

## お花摘み, お伺いを立てて — the polite prefix is left outside

`お` + `花摘み`, and `お` + `伺いを立て` + `て`. The join finds the rest and
conjugates it right, but only from the second token onward: `お` + X is tried
first, finds nothing, and the honorific stays a separate 接頭辞 token.

Both join paths require the run to *begin* on a content word (`spellable`'s head
check, and `opens_on_a_word`). A trailing 接尾辞 is admitted; a leading 接頭辞 is
not, so a prefix-initial compound Sudachi does not already hold whole can never
be rejoined.

The cause is general — お is productive on any 動作名詞 (お伺い, お願い, お答え)
and no dictionary will list every combination — so the join needs to try the run
*without* a leading honorific and re-attach it, rather than looking up お+X. It
rarely bites: お見舞い, お節介, お手上げ, ご機嫌斜め, 大慌て, 真っ最中, ど真ん中
all arrive whole from Sudachi.

## 満足げ, 悲しげ, 不安げ, 悔しげ — never joined

Left as `満足` + `げ`, because no segmentation dictionary lists the compound as a
headword. The joined ones (得意げ, 意味ありげ) work — `3523cad` made a suffix
compound take the class its suffix derives.

Open question: whether げ should be a productive suffix *rule* rather than a
dictionary lookup. Same shape as 感 below.

## 砂粒 — held whole, read さりゅう where the line means すなつぶ

The `non-word` half is gone — a standard dictionary decides wordhood now, so
砂粒 is clickable and defined. What is left is the reading: Sudachi says さりゅう
and Jitendex says すなつぶ, and only the master may name an identity, so nothing
overrules it. The general form is a reading only a `reference` dictionary knows.

## 集る — read たかる where the line means あつまる

「旧教徒の集る場所でもある」 means the believers *gather*. Every segmentation
authority lists 集る only as たかる (swarm, mob) and 集まる only as あつまる, so
Sudachi reads the surface たかる and the ladder stops at *Exact match*.
Jitendex is the only dictionary that lists the 集る/あつまる pair, and only the
master may name an identity, so nothing overrules たかる. Same family as 砂粒:
a reading only a `reference` dictionary knows. One sighting so far.

## あてぃし — a character-voice pronoun Sudachi shreds, and a fragment lands on 羊歯

「あーあてぃしだよー？」 is the childish first-person pronoun あてぃし (a drawn-out
あたし), a character's voice. The docs already list these pronoun spellings as
not vocabulary — but the truth is worse than an off-scale spelling. Sudachi's
Mode C segments あてぃし as あーあ + て + ぃ + しだ, and recompose cannot rescue
it because no listed headword combines those parts.

The fragment that costs is しだ: the master lists 羊歯 read しだ (the fern), so
the gate admits it and しだ becomes a ledger row keyed on a word the reader never
meant. て is normalised onto って, あーあ is a real interjection. The shredding
is the defect, and it is one the blacklist lever the docs name for the pronoun
spellings cannot reach — しだ is not a character-voice spelling, it is a real
master headword asserted falsely.

## The spelling class — a kanji identity for a line written in kana

とうもろこし keyed as 玉蜀黍, いびき as 鼾, ご祝儀 as 御祝儀, のしあがる as
伸し上がる, 一人暮らし as 独り暮らし. The pair *is* listed, so the exact-match
rung fires and the "would add kanji the text did not use" guard — which sits at
the bottom of the ladder — is never reached.

**Deliberately left open.** Refusing the fold would take these words off the
master scale, since いびき is not a Sankoku headword and 鼾 is; that is a
decision about the denominator, not a parse fix, and it is the same mechanism
that makes いう and 言う one row. The mechanical test for the whole category is
in the random-sample audit below: *neither the headword nor its reading occurs
in the line the token came from*.

## Short kana that is not a word at all — HALF FIXED

**The half no dictionary backs is fixed.** A kana term of at most three morae
that no dictionary lists — asked in both alphabets — is refused a ledger row
entirely (`Wordhood::is_noise`). 130 rows and 338 encounters of sound effect and
hook shrapnel: ぎい, ぐっ, ぎっ, ちゅぷ, くちゅ, ちゅる, ががが, ズチュ, グチョ.

**The rule keys on the dictionaries, so it sharpens as they are added.**
Installing 擬音語・擬態語辞典 (講談社, 1,967 headwords, 804 of them in nothing
else already here) took 15 terms straight back out of the noise bucket — きっ,
にっ, がばり, ぷん, きゅっ, ぼっ, くっ, ぷい, ざらり, そっ, ばっ, ぐっ, ぎっ,
だっ, のっ are mimetics with entries, and the rule was over-broad on them for
exactly as long as nothing listed them. ぎい, ちゅぷ, ズチュ and ががが stayed
refused.

Three morae is where the population turns. Below it a kana string nothing lists
is almost always noise; at four and five it is the work's own vocabulary —
ダイイング, トレデキム, ジンザイ, ハルウリ, ヒトカリ are what one VN is *about*
and no dictionary will ever list them. **The alphabet is asked both ways before
the string is condemned**, which is what keeps ウチ (107), コイツ, ガッコ, ソレ,
ソッチ, ミライ and シケイ: a katakana spelling is a decision about how to write a
word, not evidence that there is no word.

**What is left is the loud half, and it is loud because a dictionary lists it.**
ちゅ ×207 is in 明鏡, ぢ ×52 in Jitendex — both are one mora and both head the
work's triage queue. The rule above cannot reach them by construction, and the
lever that can is one the ledger already has: blacklisting, which drops a term
out of `top_unknown` for good.

The rule that *would* reach them is the identity ladder's own argument at the
wordhood gate: **one mora of kana is never a word of its own**, whatever the
dictionaries say, because Japanese has a word for every single kana and the
match is found every time. Measured over the ledger: 46 rows and 1,378
encounters — ちゅ, ぢ, ひ ×358, う ×213, く ×153, あ ×140 — against 37 rows and
149,058 encounters that must survive it, which are the particles (を, は, が, に)
and the affixes (さ, お, ご, め). Fencing on those two parts of speech is what
separates them. Not built: it overrules a dictionary, and it changes what the
reader paints as well as what the ledger holds.

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
  (`CLAUSE_INITIAL_ONLY`); とはいえ, なんでも, 確かに, お経 and たまえ are still
  refused, each by a different fence. See *とはいえ, 確かに, たまえ*.
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
