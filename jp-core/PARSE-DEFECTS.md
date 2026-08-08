# Parse defects

Words noticed misparsed while reading, to be worked through in a batch. One
entry per word: what the pipeline does with it, and the cause where it is known.

Check one with `#tokenize`, or:

```
curl -s localhost:3200/api/tokenize -H 'content-type: application/json' \
  -d '{"text":"…"}'
```

---

## 断腸の思い — dropped as a name

`excluded: "name"`. Segmentation and identity are both right; SudachiDict
itself tags the entry `名詞,固有名詞,一般`, and the highlighter drops every
proper noun before it consults the ledger, so a master headword mis-tagged
this way is invisible. Specific to this entry — 一期一会, 弱肉強食, 藪の中,
高嶺の花 are all fine.

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
