#!/usr/bin/env python3
"""Build the public demo's seed databases from a copy of the live ones.

    scripts/make-demo-data.py [--live-dir DIR] [--out DIR]

The demo at demo.kotodex.com serves someone else's reading history, so the
question this script answers is what may be shown to strangers. The rule is
flat rather than selective:

  **No text that came from outside this repository survives.**

Every column holding a work's title, a hooked line, a book, a mined card or a
looked-up word is overwritten — not filtered, not sampled, not checked against
a blocklist. What is kept is the *shape*: every timestamp, character count,
session boundary, encounter count and status, so the dashboard's pacing,
streaks and speed curves are a real reader's and not a random walk.

Replacement text is generated from two sources that are ours to publish: the
sentence bank below, written for this script, and the common end of the
frequency list, which is a list of ordinary words rather than anyone's prose.

The dictionary cache is emptied. The dashboard never joins it — `vocabulary`
carries its own `in_master` / `in_name` / `in_reference` flags — and it is 1.7
GB of imported dictionaries that are not ours to redistribute. The tables stay,
so a stray query returns nothing instead of failing.

Deterministic: the same input gives the same output, so regenerating the seed
after a schema change does not reshuffle every word in it.
"""

import argparse
import hashlib
import random
import sqlite3
import sys
from pathlib import Path

SEED = 20260824

# Invented titles, matched to what each work is (a long VN, a short one, a
# book) so the library page keeps its variety.
WORK_TITLES = {
    "素晴らしき日々～不連続存在～": "星降る丘の図書館",
    "ドーナドーナ いっしょにわるいことをしよう": "ねこまち商店街",
    "魔法少女ノ魔女裁判": "竜と灯台守",
    "euphoria": "春をさがす旅",
    "白昼夢の青写真": "海辺のカフェテラス",
    "夏ノ鎖": "秋風のスケッチ",
    "嫌われる勇気": "やさしい日本語入門",
    "会話の0.2秒を言語学する": "ことばの散歩道",
}

# Written for this script. Ordinary declarative sentences, no punctuation, so
# that a line built from them counts the same under
# `jp_core::text::chars::count_chars` as the row it replaces.
SENTENCES = [
    "朝の光が窓から差し込んでいた",
    "駅前の商店街はいつもより静かだった",
    "彼女は小さな声で名前を呼んだ",
    "犬が坂道をゆっくり登っていく",
    "台所からいい匂いがしてきた",
    "今日は一日中雨が降るらしい",
    "図書館の二階には誰もいなかった",
    "川の水はとても冷たかった",
    "子供たちは公園で笑っていた",
    "小さな鈴の音が遠くから聞こえる",
    "机の上に古い手紙が置いてある",
    "先生はゆっくりと黒板に字を書いた",
    "夜空には星がたくさん出ていた",
    "彼は何も言わずに立ち上がった",
    "電車は五分ほど遅れて到着した",
    "春になれば桜がきれいに咲くだろう",
    "母が作った味噌汁の味を思い出す",
    "海の向こうに小さな島が見える",
    "その本はもう何度も読み返している",
    "風が強くて帽子が飛ばされそうだ",
    "喫茶店の窓際の席が空いていた",
    "彼女は少し困った顔をしていた",
    "山の上から町全体が見下ろせる",
    "新しい仕事はまだ慣れていない",
    "夏の夕方には花火の音が響く",
    "猫は日なたで気持ちよさそうに眠る",
    "約束の時間まであと十分ある",
    "誰かが階段を上がってくる音がした",
    "この道をまっすぐ行けば駅に着く",
    "彼の言葉の意味がよく分からなかった",
    "冬の朝は布団から出るのがつらい",
    "手紙の最後に小さく名前が書いてあった",
    "店の主人は優しく笑ってくれた",
    "教室の窓から校庭がよく見える",
    "長い一日がようやく終わった",
    "彼女は約束を必ず守る人だった",
    "雪が積もって町が白くなっている",
    "その話を聞いて少し安心した",
    "人の多い場所では静かにしよう",
    "遠くの空が少しずつ明るくなってきた",
]

# Invented cast, drawn on for `work_names`.
NAMES = [
    ("春香", "はるか"), ("蓮", "れん"), ("葵", "あおい"), ("陽向", "ひなた"),
    ("千尋", "ちひろ"), ("湊", "みなと"), ("結衣", "ゆい"), ("悠真", "ゆうま"),
    ("咲良", "さくら"), ("大地", "だいち"), ("美月", "みつき"), ("颯太", "そうた"),
    ("琴音", "ことね"), ("拓海", "たくみ"), ("紗英", "さえ"), ("和樹", "かずき"),
    ("澪", "みお"), ("翔", "かける"), ("柚希", "ゆずき"), ("直人", "なおと"),
    ("小春", "こはる"), ("小夜", "さよ"), ("彰", "あきら"), ("真昼", "まひる"),
]

DICTIONARY_TABLES = [
    "dictionary_entries",
    "dictionary_frequency",
    "dictionary_pitch",
    "dictionaries",
]

# Jiten — the reader-facing frequency list, and the pool the replacement
# vocabulary is drawn from. `jp_core::knowledge::dictionaries::READER_FREQUENCY`.
POOL_DICTIONARY = "Jiten"
# Ordinary vocabulary reaches well past this; the cap is what keeps the pool to
# words a learner would recognise rather than the long tail.
POOL_MAX_RANK = 30000


def backup(src: Path, dst: Path) -> None:
    """Copy through SQLite rather than the filesystem: the live database is
    being written to, and a WAL mid-write copies as a torn file."""
    dst.unlink(missing_ok=True)
    for extra in (".db-wal", ".db-shm"):
        Path(str(dst) + extra).unlink(missing_ok=True)
    source = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
    target = sqlite3.connect(dst)
    with target:
        source.backup(target)
    source.close()
    target.close()


def word_pool(db: sqlite3.Connection) -> list[tuple[str, str]]:
    rows = db.execute(
        """SELECT DISTINCT f.term, COALESCE(NULLIF(f.reading, ''), f.term)
             FROM dictionary_frequency f
             JOIN dictionaries d ON d.id = f.dictionary_id
            WHERE d.title = ? AND f.frequency <= ?
            ORDER BY f.frequency, f.term""",
        (POOL_DICTIONARY, POOL_MAX_RANK),
    ).fetchall()
    if not rows:
        sys.exit(
            f"no {POOL_DICTIONARY} frequency rows — the live knowledge.db must "
            "have the reader frequency dictionary imported to draw a pool from"
        )
    return [(t, r) for t, r in rows]


class Words:
    """A stable replacement for every (headword, reading) the ledger holds.

    One mapping shared by every table, so a word mined in `anki_notes`, counted
    in `word_days` and judged in `vocabulary` stays the same word — the
    dashboard cross-references all three and a per-table shuffle would show a
    mined word that was never read.
    """

    def __init__(self, pool: list[tuple[str, str]], rng: random.Random):
        self.pool = pool[:]
        rng.shuffle(self.pool)
        self.next = 0
        self.by_key: dict[tuple[str, str], tuple[str, str]] = {}
        self.by_headword: dict[str, tuple[str, str]] = {}

    def get(self, headword: str, reading: str | None) -> tuple[str, str]:
        key = (headword or "", reading or "")
        if key not in self.by_key:
            word = self.pool[self.next % len(self.pool)]
            self.next += 1
            self.by_key[key] = word
            self.by_headword.setdefault(headword or "", word)
        return self.by_key[key]

    def headword(self, headword: str) -> str:
        """For the tables carrying a spelling but no reading to key on."""
        if headword not in self.by_headword:
            self.by_headword[headword] = self.get(headword, None)
        return self.by_headword[headword][0]


def line_text(rng: random.Random, chars: int) -> str:
    """A line of exactly `chars` characters, so `lines.chars` still holds."""
    if chars <= 0:
        return ""
    out = ""
    while len(out) < chars:
        out += SENTENCES[rng.randrange(len(SENTENCES))]
    return out[:chars]


def scrub_knowledge(db: sqlite3.Connection, rng: random.Random) -> None:
    pool = word_pool(db)
    words = Words(pool, rng)

    def title(work: str | None) -> str | None:
        if work is None:
            return None
        return WORK_TITLES.get(work, work)

    unmapped = {
        w for (w,) in db.execute("SELECT DISTINCT title FROM works")
    } - set(WORK_TITLES)
    if unmapped:
        sys.exit(
            "works with no invented title: "
            + ", ".join(sorted(unmapped))
            + "\nadd them to WORK_TITLES — a real title must not reach the demo"
        )

    # Works. The cover file names are kept and the images themselves replaced
    # by make-demo-covers, which draws them from the title.
    for wid, old in db.execute("SELECT id, title FROM works").fetchall():
        db.execute(
            "UPDATE works SET title = ?, vn_window = NULL WHERE id = ?",
            (WORK_TITLES[old], wid),
        )

    # Lines. Text, its wrapped form and its ruby all go; `ts`, `chars`,
    # `discarded` and the work stay, which is the whole reading history.
    rows = db.execute("SELECT id, chars, work FROM lines").fetchall()
    db.executemany(
        "UPDATE lines SET text = ?, wrapped = NULL, ruby = NULL, work = ? WHERE id = ?",
        [(line_text(rng, chars), title(work), lid) for lid, chars, work in rows],
    )

    # Manual sessions: the pasted text a book session carries.
    rows = db.execute("SELECT id, chars, work, content FROM manual_sessions").fetchall()
    db.executemany(
        "UPDATE manual_sessions SET content = ?, note = NULL, url = NULL, work = ? WHERE id = ?",
        [
            (line_text(rng, chars) if content else None, title(work), sid)
            for sid, chars, work, content in rows
        ],
    )

    # Books hold a whole flattened epub. Every position is a byte offset into
    # it, so the replacement is generated to length and the offsets scaled.
    for work, body_chars, body_start, position in db.execute(
        "SELECT work, body_chars, body_start, position FROM books"
    ).fetchall():
        text = line_text(rng, (body_start or 0) + (body_chars or 0))
        db.execute(
            """UPDATE books
                  SET work = ?, text = ?, text_bytes = ?, position = ?
                WHERE work = ?""",
            (
                title(work),
                text,
                len(text.encode()),
                min(position or 0, len(text.encode())),
                work,
            ),
        )

    db.executemany(
        "UPDATE anki_notes SET vocab = ?, headword = ? WHERE note_id = ?",
        [
            (words.headword(vocab or ""), words.headword(headword or ""), nid)
            for nid, vocab, headword in db.execute(
                "SELECT note_id, vocab, headword FROM anki_notes"
            ).fetchall()
        ],
    )

    db.executemany(
        "UPDATE lookups SET term = ?, headword = ?, work = ? WHERE id = ?",
        [
            (words.headword(term or ""), words.headword(headword or ""), title(work), lid)
            for lid, term, headword, work in db.execute(
                "SELECT id, term, headword, work FROM lookups"
            ).fetchall()
        ],
    )

    # The keyed tables are rewritten wholesale: the replacement of two rows can
    # collide on one key, so they are collapsed rather than updated in place.
    rekey(
        db,
        "vocabulary",
        ["headword", "reading"],
        lambda row: words.get(row["headword"], row["reading"]),
    )
    rekey(
        db,
        "word_days",
        ["lemma", "date"],
        lambda row: (words.headword(row["lemma"]), row["date"]),
        sum_column="count",
    )
    rekey(
        db,
        "work_terms",
        ["headword", "reading", "work"],
        lambda row: (*words.get(row["headword"], row["reading"]), title(row["work"])),
        sum_column="count",
    )
    rekey(
        db,
        "work_script_terms",
        ["headword", "reading", "work"],
        lambda row: (*words.get(row["headword"], row["reading"]), title(row["work"])),
        sum_column="count",
    )
    rekey(
        db,
        "term_surfaces",
        ["headword", "reading", "surface"],
        lambda row: (
            *words.get(row["headword"], row["reading"]),
            words.headword(row["headword"]),
        ),
        sum_column="count",
    )

    # Rebuilt from the script files, which the demo has none of.
    db.execute("DELETE FROM work_scripts")

    db.executemany(
        "UPDATE vocabulary_events SET headword = ?, reading = ? WHERE id = ?",
        [
            (*words.get(headword, reading), eid)
            for eid, headword, reading in db.execute(
                "SELECT id, headword, reading FROM vocabulary_events"
            ).fetchall()
        ],
    )

    # The cast: invented, dealt out per work so a name is still a name. Keyed
    # on (work, name), so two originals can collapse onto one invented name and
    # the table has to be rewritten rather than updated.
    dealt: dict[tuple[str, str], str] = {}
    for i, (work, name) in enumerate(
        db.execute("SELECT work, name FROM work_names ORDER BY work, name").fetchall()
    ):
        dealt[(work, name)] = NAMES[i % len(NAMES)][0]
    rekey(
        db,
        "work_names",
        ["work", "name"],
        lambda row: (title(row["work"]), dealt[(row["work"], row["name"])]),
    )

    for table in DICTIONARY_TABLES:
        db.execute(f"DELETE FROM {table}")


def rekey(db, table, key_columns, remap, sum_column=None):
    """Rewrite a table whose primary key is being replaced.

    Two distinct words can map onto one replacement, which an in-place UPDATE
    would fail on. The rows are read out, merged on the new key — summing
    `sum_column` where given, keeping the first row otherwise — and written
    back.
    """
    db.row_factory = sqlite3.Row
    rows = db.execute(f"SELECT * FROM {table}").fetchall()
    if not rows:
        db.row_factory = None
        return
    columns = list(rows[0].keys())
    merged: dict[tuple, dict] = {}
    for row in rows:
        record = dict(row)
        new_key = remap(record)
        for column, value in zip(key_columns, new_key):
            record[column] = value
        key = tuple(record[c] for c in key_columns)
        if key in merged and sum_column:
            merged[key][sum_column] += record[sum_column] or 0
        elif key not in merged:
            merged[key] = record
    db.row_factory = None
    db.execute(f"DELETE FROM {table}")
    placeholders = ",".join("?" for _ in columns)
    db.executemany(
        f"INSERT INTO {table} ({','.join(columns)}) VALUES ({placeholders})",
        [tuple(r[c] for c in columns) for r in merged.values()],
    )


# Where a Noto CJK face is likely to be. The covers carry Japanese titles, so a
# Latin-only fallback would draw a column of empty boxes.
FONT_CANDIDATES = [
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
]

COVER_SIZE = (600, 900)


def find_font() -> str:
    for path in FONT_CANDIDATES:
        if Path(path).exists():
            return path
    sys.exit(
        "no Noto CJK font found — install one, or the covers draw the titles as "
        "empty boxes"
    )


def make_covers(db: sqlite3.Connection, out: Path) -> None:
    """Draw a cover per work from its invented title.

    The real art is a publisher's, so it cannot be shipped, and a library page
    of grey rectangles is not the page being demonstrated. Colour comes from the
    title, so a work keeps its cover across regenerations.
    """
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        sys.exit("Pillow is needed to draw the demo covers: pip install pillow")

    font_path = find_font()
    out.mkdir(parents=True, exist_ok=True)
    width, height = COVER_SIZE

    for title, cover in db.execute(
        "SELECT title, cover_path FROM works WHERE cover_path IS NOT NULL"
    ).fetchall():
        digest = hashlib.sha256(title.encode()).digest()
        hue = digest[0] / 255.0
        image = Image.new("RGB", (width, height))
        draw = ImageDraw.Draw(image)

        import colorsys

        top = colorsys.hsv_to_rgb(hue, 0.45, 0.55)
        bottom = colorsys.hsv_to_rgb((hue + 0.08) % 1.0, 0.55, 0.22)
        for y in range(height):
            t = y / height
            draw.line(
                [(0, y), (width, y)],
                fill=tuple(
                    int(255 * (top[i] * (1 - t) + bottom[i] * t)) for i in range(3)
                ),
            )

        draw.rectangle([40, 40, width - 40, height - 40], outline=(255, 255, 255, 60))

        size = 58
        font = ImageFont.truetype(font_path, size)
        # Japanese wraps anywhere, so the title is broken to fit the box rather
        # than on spaces.
        per_line = max(1, (width - 160) // size)
        lines = [title[i : i + per_line] for i in range(0, len(title), per_line)]
        block = len(lines) * (size + 16)
        y = (height - block) // 2
        for line in lines:
            box = draw.textbbox((0, 0), line, font=font)
            draw.text(
                ((width - (box[2] - box[0])) // 2, y),
                line,
                font=font,
                fill=(255, 255, 255),
            )
            y += size + 16

        image.save(out / cover, quality=88)

    print(f"drew {len(list(out.glob('*.jpg')))} covers")


def scrub_stats(db: sqlite3.Connection) -> None:
    current = db.execute(
        "SELECT value FROM settings WHERE key = 'current_work'"
    ).fetchone()
    if current:
        db.execute(
            "UPDATE settings SET value = ? WHERE key = 'current_work'",
            (WORK_TITLES.get(current[0], ""),),
        )
    # The machine the reading happened on: a window title, an AnkiConnect
    # address, a WebSocket the demo has no logger for.
    db.execute(
        """UPDATE settings SET value = ''
            WHERE key IN ('vn_window', 'anki_source', 'line_source_ws_url')"""
    )
    db.execute("DELETE FROM settings WHERE key = 'vn_logger_heartbeat'")
    # VNDB ids: the covers are replaced, so the ids that fetched them would
    # only invite a re-fetch of the real art.
    db.execute("DELETE FROM work_covers")


def verify(out: Path, live_knowledge: Path) -> None:
    """Fail rather than ship a database still holding the reader's own text."""
    live = sqlite3.connect(f"file:{live_knowledge}?mode=ro", uri=True)
    demo = sqlite3.connect(out / "knowledge.db")

    originals = {t for (t,) in live.execute("SELECT DISTINCT title FROM works")}
    leaked = [
        t
        for (t,) in demo.execute("SELECT DISTINCT title FROM works")
        if t in originals
    ]
    if leaked:
        sys.exit(f"work titles survived: {leaked}")

    sample = {
        t
        for (t,) in live.execute(
            "SELECT text FROM lines WHERE text IS NOT NULL ORDER BY id LIMIT 2000"
        )
    }
    hits = [
        t
        for (t,) in demo.execute("SELECT text FROM lines WHERE text IS NOT NULL")
        if t in sample
    ]
    if hits:
        sys.exit(f"{len(hits)} original lines survived")

    for table in DICTIONARY_TABLES:
        (count,) = demo.execute(f"SELECT count(*) FROM {table}").fetchone()
        if count:
            sys.exit(f"{table} still holds {count} rows")

    live.close()
    demo.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--live-dir",
        type=Path,
        default=Path.home() / ".local/share/kotodex",
        help="where the live databases are",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "target/demo-data",
        help="where to write the seed",
    )
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(SEED)

    for name in ("knowledge.db", "read-stats.db"):
        source = args.live_dir / name
        if not source.exists():
            sys.exit(f"no {name} in {args.live_dir}")
        backup(source, args.out / name)
        print(f"copied {name}")

    knowledge = sqlite3.connect(args.out / "knowledge.db")
    with knowledge:
        scrub_knowledge(knowledge, rng)
    knowledge.execute("VACUUM")
    make_covers(knowledge, args.out / "covers")
    knowledge.close()
    print("scrubbed knowledge.db")

    stats = sqlite3.connect(args.out / "read-stats.db")
    with stats:
        scrub_stats(stats)
    stats.execute("VACUUM")
    stats.close()
    print("scrubbed read-stats.db")

    verify(args.out, args.live_dir / "knowledge.db")
    for name in ("knowledge.db", "read-stats.db"):
        size = (args.out / name).stat().st_size / 1e6
        print(f"{name}: {size:.1f} MB")
    print(f"seed written to {args.out}")


if __name__ == "__main__":
    main()
