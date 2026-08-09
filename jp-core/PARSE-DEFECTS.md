# Parse defects

Words noticed misparsed while reading, to be worked through in a batch. One
entry per word: what the pipeline does with it, and the cause where it is known.

Check one with `#tokenize`, or:

```
curl -s localhost:3200/api/tokenize -H 'content-type: application/json' \
  -d '{"text":"…"}'
```

---

## 断腸の思い, 机上の空論 — dropped as a name

`excluded: "name"`. Segmentation and identity are both right; SudachiDict
itself tags the entry `名詞,固有名詞,一般`, and the highlighter drops every
proper noun before it consults the ledger, so a master headword mis-tagged
this way is invisible. Not general to の-phrases — 一期一会, 弱肉強食, 藪の中,
高嶺の花 are all fine; it is per-entry.

Possible fix: don't trust `固有名詞` when the master lists the term as an
ordinary headword. Touches the name gate, which exists to keep a VN's cast out
of the feed.

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

## 聞きかじり — nominalized verb, master lists only the verb

Sudachi holds it whole (`名詞`), so nothing is split. It is `non-word` because
Sankoku lists 聞き齧る and not the 連用形 noun. 明鏡 lists both 聞きかじり and
聞き齧り, but a standard dictionary decides wordhood, never the vocabulary
scale.

The class: a 連用形 noun the master only carries as a verb.

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

## スッとする — mimetic split, then the と taken by とする

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

## 書き込み — same class as 聞きかじり

Sudachi holds it whole and the reading is right. `non-word` because Sankoku
lists only 書き込む; 明鏡 has both. A common word, so this class is not a tail
case — a 連用形 noun the master carries only as a verb needs a rule, not an
entry each.

## いいんだよって — the join pass builds よって out of particle + quotative

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

## 蠱毒 — only Jitendex has it

Held whole, reading こどく right. `non-word` because Jitendex (`reference`) is
the only dictionary carrying it — Sankoku, 明鏡 and 小学館 all lack it
entirely.

Different from 砂粒: that one is a transparent compound the master could be
expected to omit. This is a genuine gap — a rare literary word no dictionary of
the master's size holds, and no rule will recover it. The honest outcome for
this class may be a fourth state between `non-word` and a scale entry: known to
exist, not on the count.
