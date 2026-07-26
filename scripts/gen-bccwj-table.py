"""Reduce NINJAL's BCCWJ character table to the per-kanji rows jp-core needs.

The source workbook is the character-frequency list published with the Balanced
Corpus of Contemporary Written Japanese, downloaded by hand from
https://pj.ninjal.ac.jp/corpus_center/bccwj/freq-list.html — it is not in the
repo, only the reduction of it is. Run this again if the list is ever revised:

    scripts/gen-bccwj-table.py BCCWJ_CharacterTable.xlsx \\
        jp-core/src/text/bccwj_data.rs

Openpyxl is not a dependency here; an xlsx is a zip of XML and this needs three
columns of one sheet.
"""

import sys
import zipfile
from xml.etree import ElementTree as ET

NS = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"

src, dst = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(src)
shared = [
    "".join(t.text or "" for t in si.iter(NS + "t"))
    for si in ET.fromstring(z.read("xl/sharedStrings.xml"))
]
sheet = ET.fromstring(z.read("xl/worksheets/sheet1.xml"))


def val(cell):
    v = cell.find(NS + "v")
    if v is None:
        return ""
    return shared[int(v.text)] if cell.get("t") == "s" else v.text


rows = [
    [val(c) for c in r.findall(NS + "c")]
    for r in sheet.find(NS + "sheetData").findall(NS + "row")[1:]
]
RANGES = [
    (0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF),
    (0x20000, 0x2A6DF), (0x2A700, 0x2EE5D), (0x30000, 0x33479),
]


def is_kanji(c):
    """Mirror jp_core::text::kanji::is_kanji — 々〇 are 漢 to NINJAL but stand
    in for a kanji rather than being one, and the grid never shows them."""
    return any(lo <= ord(c) <= hi for lo, hi in RANGES)


# 字種 == 漢 is the table's own verdict on what is a kanji; trust it, but keep
# only single codepoints in the ranges the rest of jp-core agrees are kanji.
kanji = [
    (r[0], int(r[3]))
    for r in rows
    if len(r) > 3 and r[2] == "漢" and len(r[0]) == 1 and is_kanji(r[0])
]
kanji.sort(key=lambda kv: (-kv[1], kv[0]))
total = sum(n for _, n in kanji)
rank = {c: i + 1 for i, (c, _) in enumerate(kanji)}
kanji.sort(key=lambda kv: ord(kv[0]))

out = [
    "//! Generated BCCWJ kanji frequency table — do not edit by hand.",
    "//!",
    "//! Source: the character-frequency list of the Balanced Corpus of",
    "//! Contemporary Written Japanese (BCCWJ), published by NINJAL, reduced to",
    "//! its 漢 rows. BCCWJ is a hundred-million-word balanced corpus — books,",
    "//! magazines, newspapers, white papers, blogs and Q&A — which is why it is",
    "//! the yardstick here rather than a single-genre count: its head has both",
    "//! 見思言 and 年国会 in it, where a Wikipedia count is all dates and place",
    "//! names and an Aozora count is all pre-war fiction.",
    "//!",
    "//! Sorted by codepoint so [`super::kanji::bccwj`] can binary-search it.",
    "",
    "/// `(kanji, occurrences, rank)`. `rank` is 1-based among the kanji of the",
    "/// corpus, 1 = commonest. Occurrences are raw counts over"
    f" {total:_} kanji",
    "/// in total, which is what makes a share comparable across the table.",
    "pub static BCCWJ: &[(char, u32, u16)] = &[",
]
for c, n in kanji:
    out.append(f"    ({c!r}, {n}, {rank[c]}),")
out.append("];")
out.append("")
out.append("/// Total kanji occurrences in the corpus — the denominator for a share.")
out.append(f"pub const BCCWJ_TOTAL: u64 = {total};")
out.append("")
open(dst, "w").write("\n".join(out))
print(f"{len(kanji)} kanji, {total} occurrences -> {dst}")
