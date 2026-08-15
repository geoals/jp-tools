# Parse defects

Words noticed misparsed while reading, to be worked through in a batch. One
entry per word: what the pipeline does with it, and the cause where it is known.

Check one with `#tokenize`, or:

```
curl -s localhost:3200/api/tokenize -H 'content-type: application/json' \
  -d '{"text":"…"}'
```

---

## 断腸の思い, 机上の空論 — dropped as a name — FIXED

`excluded: "name"`. Segmentation and identity are both right; SudachiDict
itself tags the entry `名詞,固有名詞,一般`, and the highlighter drops every
proper noun before it consults the ledger, so a master headword mis-tagged
this way is invisible. Not general to の-phrases — 一期一会, 弱肉強食, 藪の中,
高嶺の花 are all fine; it is per-entry.

**Fixed**, but not the way this entry proposed. "Don't trust `固有名詞` when
the master lists the term" is far too wide: 橘, 出雲, 葵, 司, 水上, デンマーク,
孔子 and シェリー are all master headwords and all names, and admitting them is
the whole thing the gate exists to prevent — 4,172 occurrences of it in this
corpus.

**Mixed script is what separates the two**, and nothing else tried does. A
Japanese name is written in kanji or in katakana; it does not carry okurigana.
`SudachiTokenizer::ordinary_headword` is that rule, applied where `proper_noun`
is *set* rather than where the highlighter reads it, so ingest's proper-noun
ratio and the reader's tint cannot disagree.

Over the corpus it admits 63 occurrences of 16 terms — もう少し, 何となく,
相変わらず, 鳥肌が立つ, 目立ちたがり, 魔法使い, 高みの見物, 陸の孤島,
知る権利, 無茶振り, 棒高跳び, 悪魔の証明, 上から目線, ドミノ倒し and the two
above — every one of them vocabulary, and moves no name at all.

What it does not reach is the same defect on a term with no okurigana: 予定調和,
悪魔, 王子, 城, 金, 鏡 are all mis-tagged the same way and are indistinguishable
from a name by any signal available here.

**Those six and 眸 are now named, in `NOT_A_NAME`** — one reviewed judgement per
string, the same shape as `NEVER_JOIN`, since nothing structural separates them
from 橘 or 葵. 36 occurrences over the corpus stop being dropped. A work that
really does have a character called 悪魔 says so in its cast list, and the cast
list is asked first.

## 満足げ, 悲しげ, 不安げ, 悔しげ — never joined

Left as `満足` + `げ`, because no segmentation dictionary lists the compound as
a headword. The joined ones (得意げ, 意味ありげ) now work — see
`3523cad`, which made a suffix compound take the class its suffix derives.

Open question: whether げ should be a productive suffix *rule* rather than a
dictionary lookup.

## お花摘み — split at the prefix, and unlistable

Two stacked causes.

1. Only Jitendex lists 花摘み and お花摘み, and its role is `reference`, so no
   segmentation authority can admit the join and the master cannot rank
   花摘み — hence `non-word`.
2. Sudachi splits お off as `接頭辞`, and both join paths require the run to
   *begin* on a content word (`spellable`'s head check, and `opens_on_a_word`).
   A trailing `接尾辞` is admitted; a leading `接頭辞` is not. So a
   prefix-initial compound Sudachi does not already hold whole can never be
   rejoined.

(2) rarely bites — お見舞い, お節介, お手上げ, ご機嫌斜め, 大慌て, 真っ最中,
ど真ん中 all arrive whole from Sudachi. Fixing (2) alone would not fix this
word; only a role change for Jitendex would, and that is a huge blast radius.

Cause (1) is now half gone: the `Wordhood` fix at the bottom of this file makes
花摘み clickable on its own, so (2) — the leading `接頭辞` the join refuses — is
all that stands between the reader and お花摘み. Ranking it still needs the
master.

## 聞きかじり — nominalized verb, master lists only the verb — HALF FIXED

Sudachi holds it whole (`名詞`), so nothing is split. It is `non-word` because
Sankoku lists 聞き齧る and not the 連用形 noun. 明鏡 lists both 聞きかじり and
聞き齧り, but a standard dictionary decides wordhood, never the vocabulary
scale.

The class: a 連用形 noun the master only carries as a verb.

**The `non-word` half is fixed, and the premise above was wrong.** A standard
dictionary *does* decide wordhood now — it always should have, since the gate is
"lenient: any dictionary" and 明鏡 is a dictionary, already trusted with the
harder question of where a word ends. 41,645 terms were in 明鏡 or 小学館 and in
nothing else, and all of them lost their span.

聞きかじり is clickable and defined. It is still off the vocabulary scale, and
that part is not a defect: the scale is the master alone by design, and adding a
dictionary must change classification and never the denominator.

Same for 書き込み and 窪み below, and for 砂粒, 連帯感, 蠱毒, 依代 and ムワムワ,
all of which Jitendex already covered. **What is left of this whole family is one
question — whether a 連用形 noun should count on the scale when the master lists
its verb — and that is a rule about the denominator, not a parse defect.**

## 砂粒 — compound only Jitendex lists

Sudachi holds it whole (`名詞`), so nothing is split. It is `non-word` because
the only dictionary carrying it is Jitendex, whose role is `reference` — no
segmentation authority and not the master. The reading also falls back to
Sudachi's さりゅう; Jitendex says すなつぶ, which is what the line meant.

Same class as お花摘み's cause (1): a transparent compound that only the large
reference dictionary lists, so it reads as unlistable and off the scale.

## あばら家 — master lists only the kanji spelling, and the kanji guard blocks it

Segmentation is right: Sudachi holds あばら家 whole, and the standard
dictionaries list it, so the wordhood gate keeps it. It is still `non-word`
because Sankoku carries only 荒ら家 / 荒ら屋. The reading matches exactly
(あばらや, in both), but identity resolves under "kept as written, since
normalising it would add kanji the text did not use" — and that guard is what
blocks the one link that would put it on the scale.

The class: a master headword written only in kanji, met in a kana or mixed
spelling, where the reading alone would identify it. Loosening the guard is
the same knob that keeps 其れ and 此の out of the corpus, so it needs to key on
the master listing the reading unambiguously, not on the spelling.

## スッとする — mimetic split, then the と taken by とする — FIXED

Sudachi splits it as `スッ` + `と` + `する`. Sankoku lists すっと as a headword,
so the pair should rejoin, but the join of `スッ`+`と` reports
`No match: parts form no listed headword by spelling or by reading` — the
spelling is スッと and the reading candidate is スッ, neither of which is the
listed すっと. The gate does try the kana fold (`Not in a dictionary that
decides segmentation: スッ, すっ`), so the fold exists; the join pass does not
apply it.

The と is then free, and `と`+`する` joins as とする. The result is a
`non-word` すっ followed by a grammar expression, with the mimetic word gone.

Two things to fix, in order: fold katakana to hiragana when the join pass looks
a run up, and check whether a longer join starting earlier should beat とする
when both are available.

**Fixed by the first alone.** The join now offers the run folded to hiragana,
last and only for an all-kana run that opens on a content word — the alphabet is
part of the spelling wherever a word is written in kanji, which is the ザル/ざる
argument, and the fence keeps の + メル from spelling のめる. Once スッ + と
joins, the と is no longer free and とする never forms, so the second question
did not have to be answered.

62 occurrences recovered corpus-wide, almost all one family: パッと, ピタリと,
ツンと, ピシャリと, ギュッと, ギクリと, ドサリと, ニコッと, ギロリと, キュッと,
ガクッと. One is wrong — モノ + の → ものの, where モノ is 物.

## 依代 — okurigana variant of a master headword

Segmentation and reading are both right (よりしろ, matching the furigana in the
line). It is `non-word` because every listing is of a *different* okurigana:
Sankoku has 依り代 only, 明鏡 has 依り代 and 憑代, and only Jitendex — role
`reference` — carries the bare 依代. So the wordhood gate drops it too.

Same family as あばら家: the master headword and the surface differ only in how
much kana is written, and the reading identifies it unambiguously. Here the
normalisation would *remove* kana rather than add kanji, so the "would add
kanji the text did not use" guard is not even the obstacle — nothing tries the
link at all.

## なんてひどい — boundary in the wrong place, and a spurious 手酷い

Sudachi Mode C returns `なん` + `てひどい`, not `なんて` + `ひどい`. Everything
downstream then confirms it: 手酷い is a real Sankoku headword reading てひどい,
so the gate keeps it and identity matches exactly. The line reads as 何 +
手酷い + 怪我.

This is worse than a `non-word`. It is a false positive on a rare word —
freq_rank 27,753, entered the ledger as `new` on this one encounter — so the
defect writes an assertion rather than just failing to make one.

The join pass cannot help: it merges adjacent tokens and never moves a
boundary. なんて and ひどい are both master headwords, so the fix has to be
either a re-segmentation check when an alternative split has better dictionary
support, or the orthographic rewrite pass handling なんて before Sudachi sees
it.

## 書き込み — same class as 聞きかじり — HALF FIXED

Sudachi holds it whole and the reading is right. `non-word` because Sankoku
lists only 書き込む; 明鏡 has both. A common word, so this class is not a tail
case — a 連用形 noun the master carries only as a verb needs a rule, not an
entry each.

## いいんだよって — the join pass builds よって out of particle + quotative — FIXED

Sudachi segments it right: `いい` `ん` `だ` `よ` `って`. The join pass then
merges the last two, because よって is a listed headword ("therefore"), and the
line acquires a rank-119,268 word entered as `new`.

A second false positive, and the mirror of なんてひどい: there Sudachi put the
boundary wrong, here Sudachi was right and the join pass removed a correct one.
Both write an assertion off one encounter.

The signal available: 名詞/接続詞 よって cannot follow the sentence-final
particle よ, and the parts are two grammar tokens. The `ん`+`だ` join was
refused a moment earlier by the length floor — which よって clears at exactly
three characters while being just as much a function-word run.

**Fixed by `NEVER_JOIN`**, which turned out to cost nothing: the real
「彼によって」 arrives whole from Sudachi and never goes through recompose, so
the only joins blocked are the two wrong ones. Five more went in with it —
も+やる, も+やっと, は+やめ, the tail of いらっしゃい, and ええ+ん, which is
crying every time. からに was looked at and left: 「するからには」 is the
construction and only one of three sightings is not.

## 蠱毒 — only Jitendex has it

Held whole, reading こどく right. `non-word` because Jitendex (`reference`) is
the only dictionary carrying it — Sankoku, 明鏡 and 小学館 all lack it
entirely.

Different from 砂粒: that one is a transparent compound the master could be
expected to omit. This is a genuine gap — a rare literary word no dictionary of
the master's size holds, and no rule will recover it. The honest outcome for
this class may be a fourth state between `non-word` and a scale entry: known to
exist, not on the count.

## 連帯感 — 感 compound the master omits

Held whole, reading れんたいかん right. `non-word` because Sankoku lacks it; a
`standard` monolingual has it, which is what keeps segmentation correct.

Same shape as 書き込み and 砂粒: a productive suffix (感 on a noun — 連帯感,
劣等感, 疎外感) that the master lists only as the bare stem. A rule for 感
compounds would do what per-entry additions cannot.

## またいちから — boundary wrong, then a rank-151,785 false positive

Sudachi Mode C returns `ま` + `たいち` + `から`, not `また` + `いち` + `から`.
The gate drops たいち, but identity then matches it *by reading only* and the
line acquires 対置 (freq_rank 151,785), entered as `new`.

Third of the boundary family, with なんてひどい and いいんだよって, and the
worst of the three: the assertion is written by the reading-only fallback on a
token the gate had already refused. A hiragana token that failed the gate
should not be able to claim a kanji headword five orders of magnitude rarer
than the words around it — a frequency floor on the reading-only rule would
stop this class without touching segmentation.

また before Sudachi, in the orthographic rewrite pass, is the same fix なんて
wants.

## ひとりもいやしない — 弥 out of the いやしない negative — FIXED

`ひとり` + `も` + `いや` + `し` + `ない`. The line is 居やしない, the emphatic
negative of いる (…や+しない), so the correct split has no いや in it at all.
いや then matches 弥 exactly — master headword, spelling and reading both — and
a rank-43,783 副詞 enters the ledger as `new`.

Fourth of the boundary family, and it defeats the fix the others suggest: this
is not a reading-only match, so a frequency floor on that rule does nothing.
The signal is grammatical — 弥 is an adverb and cannot be followed by する in
its 連用形.

The 〜やしない contraction is productive (ありゃしない, できやしない,
わかりゃしない). Handling it as a rewrite before Sudachi, like なんて and また,
covers the family in one place.

**Fixed without touching segmentation.** The split is still wrong — いや is
still there — but it no longer names 弥, because a two-mora hiragana surface may
not take a kanji identity the reader-facing list ranks worse than 15,000. That
is the one-mora rule at the next mora out, and the same argument: Japanese has a
kanji for every mora, so at two the match is still found every time and is
evidence none of them.

169 occurrences over the corpus, 90% of them wrong — 篠, 歯牙 and 使途 off the
あてぃし of a streamer's dialect, 縷々 off a sung るーるー, 河豚 off a choking
noise. What it costs is 仄, 皹, 練り, 魔羅 and 反吐, which stay as written.

Rarity is half the rule: とき, あと and はず are the same two morae and 時, 後,
筈 are simply right. Katakana is exempt — it is itself a decision about spelling,
and ハエ, キク, ツタ, カス, アザ are the words the guard would otherwise throw
away.

**The reading-only frequency floor this entry's neighbours asked for is not what
was built.** またいちから's 対置 is three morae and still gets through; a floor
wide enough to catch it also refuses そっぽ/外方 at rank 209,173, which is right.

## お伺いを立てて — the polite prefix left outside the expression

`お` + `伺いを立て` + `て`. The join pass finds the expression and conjugates
its last word right, but only from `伺い` onward: `お` + `伺い` was tried first,
found nothing, and the honorific stays a separate 接頭辞 token. The expression
is then `non-word`, since 伺いを立てる is listed by a non-master dictionary
only.

Two causes, one line. The prefix one is general — お is productive on any 動作名詞
(お伺い, お願い, お答え) and no dictionary will list every combination, so the
join needs to try the run *without* a leading honorific and re-attach it, not
look up お+X. The `non-word` one is the 連帯感/砂粒 class again: a real headword
Sankoku lacks.

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
ladder falls through to the kana surface. Fixing (1) removes this instance;
the mismatch itself is wider.

## 窪み — same class as 聞きかじり and 書き込み — HALF FIXED

Sudachi holds it whole and the reading is right. Off the master scale because
Sankoku lists only 窪む; 明鏡, 小学館 and Jitendex all have 窪み. The popup is
fine — this is the wordhood/scale question alone.

Third instance of the class, and the second common word in it. A 連用形 noun
whose verb the master lists needs a derivation rule, not one entry per word.

## ムワムワ — no span at all, because the no-row path asks only the master — FIXED

Segmentation and identity are right (`ムワムワ` + `漂い` + …), and
`/api/reader/define?term=ムワムワ` returns the Jitendex entry. The reader still
gets no popup, because there is no span to click: the token comes back
`excluded: "non-word"`.

The two wordhood tests in `highlight::classify` disagree. A term with a ledger
row is tested with `VocabRow::is_word` — `in_master || in_name || in_reference`,
the lenient gate. A term with *no* row falls to `Highlighter::in_master_lexicon`,
which is the master alone. ムワムワ is in Jitendex only, so it is a word on the
first test and a non-word on the second, and which one it gets depends on
whether ingest has run: the line was 29816 and the watermark stood at 29519, so
the reader met it live and got the strict answer. ポテチ, same class, has a row
and paints `new`.

So this is not really a parse defect — the strict test is on the wrong path.
The no-row branch should ask the same question the row does, which means the
lexicon the highlighter carries needs the reference and name dictionaries in it,
not just the master. Note the ledger row it would then create still stays off
the vocabulary scale (`COUNTS_AS_VOCAB` is `in_master`), which is correct.

The word itself is the 連帯感/砂粒 class — a real headword Sankoku lacks — with a
productive shape behind it: katakana mimetics are coined freely (ムワムワ,
ムワッと, ムンムン) and no master dictionary will list them all.

**Fixed.** `Highlighter` now carries a `Wordhood` set — the headwords *and* the
readings of the master, name and reference dictionaries — and the no-row branch
asks it, which is `VocabRow::is_word`'s question with the same kana half
`refresh_dictionary_flags` gives a row. The scale is untouched: `COUNTS_AS_VOCAB`
is still `in_master`, so these get a span and a popup and no count.

This was the general clickability defect, not one word. Anything a reference
dictionary alone lists — 景気づけ (Sankoku has only 景気付け), 花摘み, ムワムワ —
was unclickable when met live and clickable once ingest had written a row, so
whether a word could be tapped depended on a race with the watermark.

---

## 何時 — the clock reading なんじ is never produced, and the まで/でも forms become particles

Three spellings of one word, and the pipeline gets the literal one wrong every
way it can. Noticed in 白昼夢の青写真's script, where the literal string occurs
twice and neither is the set phrase:

| line | what comes out |
| --- | --- |
| 「……何時なの」 | 何時 / **なんどき**, `名詞` |
| 学校は何時までかかるだろうか。 | 何時まで / **いつまで**, `助詞` |
| いつ何時でも | いつ → 何時/いつ, then 何時でも / いつでも, `助詞` |

Both lines mean なんじ — asking the time — and no path produces it. なんどき is
a real reading but belongs to いつ何時, which is the one context here that
*doesn't* get it.

Two separate things are wrong:

- **The reading.** 何時 is いつ, なんじ or なんどき, and the choice needs the
  sentence. なんどき is the rarest of the three and is what a bare 何時 lands on.
- **The join.** 何時まで and 何時でも come back tagged `助詞` — a compound of
  noun + particle presented as one particle, with a reading (いつまで) asserted
  over the whole thing. That is why the second line contributes no 何時 row at
  all, and it is a category error rather than a close call.

Low volume and worth leaving alone until the batch: 217 of this work's 218
何時 rows are kana いつ normalized onto the master's spelling, which is the rule
working. The damage is confined to the literal spelling.

One thing to note when it is fixed, because it looks like a second defect and
is not: 何時/なんどき painted `known` on the first line, borrowed from the known
何時/いつ row by the judged-under-another-reading rule. That rule is right; it
was applied to a key that should never have been built.

---

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

---

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

---

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

## Names — 2026-08-15, one 30-minute session audited token by token

290 lines, 4,860 characters, 3,661 tokens, re-run through `/api/tokenize` —
`highlight::analyze`, the same call the feed makes. Widened to the two days of
白昼夢の青写真 read so far (1,321 lines, 16,949 tokens) where a count needed the
larger sample.

Name handling is not one defect but four, and they share a cause: **`proper_noun`
is Sudachi's per-occurrence POS subclass** (`to_token`, `tokenize.rs:1668`), so
the verdict is a property of the sentence, not of the term.

**All four are fixed by `work_names`, which was imported and never read.** The
table had been filled from VNDB and the tokenizer was never told about it, so
the note under `jp-script names` — "the tokenizer reads these on its next build"
— was a promise nothing kept. It reads them now, in three places: the gate keeps
a cast name whole, `join_names` puts back together one Sudachi split, and
`split_names` takes apart one Sudachi glued to a particle. What is below is the
account of each defect as it was found.

**ウィル — the same name both ways in the same session.** 16 occurrences
`excluded: "name"`, 11 counted as vocabulary. The traces differ only in what
Sudachi tagged: in 「――大丈夫ですよ、ウィル」 the join steps read *Blocked from
merging: contains a proper noun*; in 「えっ！　ウィルも立つの！？」 the same
surface produces *No match: parts form no listed headword*. Nothing downstream
can tell the two apart, because nothing downstream remembers the first verdict.
The ledger row exists with `encounter_count 21` and paints `seen` on every
occurrence Sudachi happened to call a common noun.

**ロブ — never a name at all.** 13 occurrences, all counted;
`encounter_count 74`, `freq_rank 44139` — Jiten ranks the tennis stroke. Two
lines below エド, which is excluded every time.

**世凪 — split, then both halves counted.** 世/よ ×22 and 凪/なぎ ×21, neither
name-tagged. The largest single leak in the window, and invisible as a name
because the split happens first.

**テンブリッジ ×9, ハーミア ×4 — dropped as `non-word`.** No dictionary lists
them, so they never reach the name gate. No span, which is the right screen
behaviour by accident; the classification is still absent.

**眸 ×4 — a false name.** 「二つの眸は閉じられている」, 「真っ赤な眸が…」 — the
common noun, used as one, excluded as a name and so never counted and never
tappable. `ordinary_headword`'s mixed-script rule cannot reach it: 眸 carries no
okurigana, exactly the gap that entry already names.

**タンバレイン → タンバ + レイン.** Both halves name-excluded, so nothing leaks;
the title 『タンバレイン大王』 just has no whole form to look up.

The fix these point at is stickiness: a name is a fact about a term, decided
once and remembered, not re-derived from each sentence. Something the ledger
could hold — `vocabulary.status = 'name'` was removed for the stated reason that
names never reach the ledger, which is exactly what is not true here.

**The cast list is that stickiness, and it comes from outside rather than from
the corpus**, which is better: a name is knowable before the work is read.
ウィル, ロブ, テンブリッジ, ハーミア, タンバレイン, ハチマル, リープ and パピー
are not in VNDB's cast — it lists ウィリアム・シェイクスピア and not the ウィル
everyone calls him — so `jp-script names <work> add` was written for them, under
its own source so a refetch cannot drop them. Full names are also split on ・ for
the tokenizer, since the script says シェイクスピア far more often than the whole.

**眸 is fixed the other way**, by `NOT_A_NAME`; see the 断腸の思い entry.

## Six non-name defects from the same session

- **うるせー → 煩い/わずらい.** 「うるせーな！」 — the colloquial うるさい is
  identified as the noun 煩い. 22 encounters on that row. **Still open, and one
  fix was tried and backed out**: preferring whichever listed reading shares an
  onset with the kana surface picks うるさい here, and over the corpus it also
  turns コイツ into 此奴/こやつ, きわまり into 極まり/きまり, まじか into 間近 and
  いーっぱい into 一杯 — a reading is too weak a signal to arbitrate a spelling
  on. What the family actually needs is the colloquial ending itself
  (〜あい → 〜えー: すげー, やべー, あぶねー, おもしれー), which is a kana
  transformation and not a similarity score. Only 煩い is wrong today, because it
  is the only one of them whose master spelling has two readings.
- **深かっ → 深い/ぶかい.** 「深かったようにも思える」 — the bound compound
  reading (奥深い) on a bare 深い, beside 浅かっ → 浅い/あさい in the same
  sentence, which is right. **FIXED**, and it was not one word: Sudachi reads a
  *standalone* 深い as ブカイ, so the ladder's re-derivation rung asked the same
  wrong oracle twice and every 深い, 深く and 深かっ in the corpus was off the
  master scale. A new rung under it takes the master's own reading when the two
  differ by nothing but the first mora's voicing — 箱/ばこ → はこ is the same
  defect, 36 occurrences, and 就く/づく → つく a third. Fenced to the voicing
  alone: 所為 is せい in the text and しょい in the master, and rewriting *that*
  asserts a different word.
- **一日 → 一日/ついたち.** 「一日手伝うだけじゃあ」 — いちにち. The
  `preferred_readings` table has no entry that reaches this.
- **いやがおうにも → いや/否 + が + お + うに + も.** 否が応にも, kana-spelt.
  Produces two junk tokens; うに is now a ledger row.
- **この家 ×48 → この家/このや.** Not in this list when it was written, and the
  same shape: 此の家 is a master headword read このや, so the join builds it out
  of この + 家 every time the text says "this house". **FIXED** — `NEVER_JOIN`.
- **牛乳粥 → 牛 + 乳粥.** Sudachi Mode C's own boundary. 牛 counted, 乳粥
  dropped `non-word`; 牛乳 in the next clause of the same line is right.
- **空の下に出る → あの + 空 + の + 下に出る/したにでる.** A join built the
  idiom out of 下 + に + 出る where the text has none. **FIXED** — `NEVER_JOIN`.
  All six sightings in the script are 廊下に出た, 真下に出た, 空の下に出る; the
  idiom that exists is 下手に出る and Sudachi hands that over whole.

Suffix splits in the same window that are working as designed and are listed
only so a later pass does not re-report them: 脚本+家, 完成+形, 密会+所,
旧教+徒, 大+受け, 海軍+大臣.

## 宣戦布告 — Mode C had it whole and the gate broke it — FIXED

```
gate  宣戦布告  kept:false  "Not in a dictionary that decides segmentation"
split 宣戦布告  mode:B      ["宣戦","布告"]
```

Sudachi's Mode C returns 宣戦布告 as one morpheme. Only Jitendex lists it, and
its role is `reference`, so the gate rejects the whole form and falls back to
the Mode B split — two ledger rows (宣戦 rank 47,197, 布告 rank 34,041) where
the text has one everyday word.

Same cause as 砂粒 and 蠱毒, but the failure mode is the opposite way round and
worth separating: those never had a whole form to keep. Here the segmentation
arrived **correct** and the gate destroyed it. A rule that trusts Mode C when it
is *more* aggressive than the fallback would fix this class without touching
the join paths.

**Fixed by the research note below, measured before it was built.** A Mode C
morpheme the gate rejects is kept anyway when the reader-facing list ranks it
above *every* part the Mode B split would produce. Over 32,353 lines it fires
three times — 宣戦布告, 無味無臭 and 掘りごたつ — and all three are one word. The
conservative form of the threshold was the right one: strictly commoner than
both halves.

## かたや — read as 方/かた

「かたやおれは、専業になって初めての作品だ」 → `かた` + `や`. The conjunction
かたや is not listed, so the ladder takes the two-mora kana run as the master
headword 方/かた on an exact spelling-and-reading match. `two_mora_coincidence`
is the rule that should catch this and does not, because the match is by
*reading* to a real headword rather than a coincidental one.

Cost is not the ledger row — it is that the popup opens on 方 and the reader has
to work out that the word was never there.

## 塵 — read ごみ where the sense is ちり

「微かな塵が混ざっているのがわかった」 → 塵/ごみ. Sudachi returns the reading
ゴミ, the master lists 塵/ごみ, and the identity ladder stops at *Exact match:
master dictionary lists both spelling and reading* with a single candidate. No
alternative is ever weighed.

The frequency tables would not have saved it — BCCWJ and Jiten both rank 塵/ごみ
far above 塵/ちり (2,661 vs 87,036 in Jiten), because the corpus writes ごみ in
kanji and modern fiction writes it ゴミ. Sankoku lists ちり first and twice.

**The consequence is ordering in the popup, not loss.** `define::definitions`
filters senses to the reading the tokenizer chose whenever the dictionary lists
it, so the first thing drawn is ごみ; the other two readings are still reachable
through the expansion chips, and the reader took them. What the wrong pick costs
is the first guess, on a spelling where Sankoku, 明鏡 and Jitendex all list three
readings and Sankoku's own order puts ちり first.

Worth considering: when a spelling has several listed readings and the choice
came from Sudachi alone rather than from a preference or a rank, lead with the
master's order instead of Sudachi's pick.

### Research note — rank the whole form against its parts before splitting

Suggested while auditing 宣戦布告, and general to the class.

The gate currently asks one question: *is the whole form listed by a dictionary
that decides segmentation?* If not, it splits. It never asks whether the split is
an improvement.

A second test that costs one lookup each: **keep Mode C's whole form when it is
commoner than its parts.** 宣戦布告 ranks 11,761 in Jiten; 宣戦 is 47,197 and
布告 34,041. A compound that outranks both halves is a word the reader meets as
one thing, whatever the segmentation dictionaries happen to list.

Two things to settle before trying it:

- It needs Jiten to rank the whole form, and Jiten ranks 430k terms against the
  master's 82k, so this admits compounds the master has no entry for. That is a
  wordhood decision made by a frequency list, which is a role change in all but
  name — the same objection that keeps Jitendex out of segmentation.
- The threshold is not obvious. Strictly commoner than both parts is the
  conservative form; commoner than the *rarer* part would be far more permissive
  and probably wrong.

Worth measuring against the corpus before it is worth building: how many joins
the rule would make, and how many of them are wrong.

---

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
凛と 72 → 0, 鯱 70 → 0. Against the read corpus it is 388 lines and nothing but
names.

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

### What was refused, and why it is worth recording

**Arbitrating a spelling by how much its reading looks like the surface.** It
fixes うるせー → 煩い/うるさい and breaks four other things in the same corpus:
コイツ → 此奴/こやつ, きわまり → 極まり/きまり, まじか → 間近, いーっぱい → 一杯.
A reading is the weakest signal there is; a similarity score over it is weaker
still.

**Taking the master's single reading whenever Sudachi's is unlisted.** The first
form of the 深い fix, and it rewrote 所為/せい to しょい and 出入/でいり to
しゅつにゅう — spellings the master merely reads some other way, which are
different words rather than one word's compound form. Voicing is a mechanical
relation and is the only one that licenses the swap.

**Keying a kana line on the master's kanji spelling.** 玉蜀黍 for とうもろこし,
鼾 for いびき, 御祝儀 for ご祝儀 — the tail defect measured earlier in this file.
Refusing the fold would move those words off the master scale, which is a
decision about the denominator and not a parse fix. It stays open deliberately.
