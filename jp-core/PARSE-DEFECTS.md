# Parse defects

Words noticed misparsed while reading, worked through in batches. One entry per
word: what the pipeline does with it today, and the cause where it is known.

**13 open**, listed below and each re-checked against the live pipeline on
2026-08-15. Three further questions are about the vocabulary denominator rather
than the parse and are kept apart from them; what has been fixed is one line
each at the bottom.

Check one with `#tokenize`, or:

```
curl -s localhost:3200/api/tokenize -H 'content-type: application/json' \
  -d '{"text":"…"}'
```

---

# Open

## ロボットがふよふよと — the particle is swallowed into an unlisted mimetic

`ロボット` + **`がふ`** + `よ` + `ふ` + `よ` + `と`. Sudachi has no ふよふよ, so
the cheapest path over 「ロボットがふよふよと」 takes the subject particle が into
a nonsense 副詞 and shreds the mimetic. Two false rows come out of it: がふ as an
adverb, and よ resolved to the adjective よい.

**No dictionary lists ふよふよ at all** — not Sankoku, not 明鏡, not Jitendex,
and not 擬音語・擬態語辞典, which was installed to check — so even a perfect
segmentation leaves it a `non-word`. What is lost is the が, and the two
assertions.

Same family as the three above, and the named list cannot take it: mimetics are
coined freely and no list will hold them. The rule that would is one keyed on
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

## うるせー — read 煩い/わずらい

The colloquial うるさい is identified as the noun 煩い. 22 encounters on that row.

**One fix was tried and backed out**: preferring whichever listed reading shares
an onset with the kana surface picks うるさい here, and over the corpus it also
turns コイツ into 此奴/こやつ, きわまり into 極まり/きまり, まじか into 間近 and
いーっぱい into 一杯. A reading is too weak a signal to arbitrate a spelling on.

What the family needs is the colloquial ending itself — 〜あい → 〜えー, which is
kana arithmetic (すげー, やべー, あぶねー, おもしれー) and not a similarity
score. Only 煩い is wrong today, because it is the only one of them whose master
spelling carries two readings.

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
| うるせー read わずらい | **tried and backed out** — see that entry | — |

### What was refused

Three rules that looked right and were measured wrong. Recorded so they are not
tried again.

- **Arbitrating a spelling by how much its reading resembles the surface** —
  see the うるせー entry.
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
- **なんてひどい → 何 + 手酷い, またいちから → ま + たいち + から, 牛乳粥 → 牛 +
  乳粥** — the boundary family, and the one class no rule over the finished
  tokens could reach: recomposition merges adjacent tokens and never moves a
  boundary. `CUT_BEFORE_AND_AFTER` is a named list of strings handed to Sudachi
  on their own, applied **only where the analysis shows the boundary actually
  came out wrong** — 14 lines over the corpus, against 59 when the cut was
  unconditional. See the entry below for why it is a list and not a rule.
