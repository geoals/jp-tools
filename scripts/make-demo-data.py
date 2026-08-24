#!/usr/bin/env python3
"""Build the public demo's seed databases from a copy of the live ones.

    scripts/make-demo-data.py [--live-dir DIR] [--out DIR]

What the demo may show splits on one line, and it is not "is this my data" but
"is this the work itself":

  **A catalogue of what was read is kept. The works themselves are not.**

Titles, covers, vocabulary, timestamps and every count are real, because a list
of what someone read is a bookshelf. The hooked lines and the books are
replaced, because `lines` is the script of a commercial work and `books.text`
is a whole epub, and the demo serves both over public GETs —
`/api/lines/before` pages back through the entire stream. Shipping those is
republishing the work rather than describing it.

The dictionary cache is emptied for the same reason, plus a practical one: it
is 1.7 GB of imported dictionaries, and the dashboard never joins it —
`vocabulary` carries its own `in_master` / `in_name` / `in_reference` flags. The
tables stay, so a stray query returns nothing instead of failing.

Two works are renamed and given drawn covers. What belongs on a public page is
a separate question from what may be published at all.

Deterministic: the same input gives the same output.
"""

import argparse
import hashlib
import random
import shutil
import sqlite3
import sys
from pathlib import Path

SEED = 20260824

# Renamed for the public page, not for copyright. Anything absent from this map
# keeps its real title.
WORK_TITLES = {
    "euphoria": "CLANNAD",
    "夏ノ鎖": "Fate/stay night",
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

# For the renamed works only: their real cast would name the work the rename
# exists to hide.
NAMES = [
    "春香", "蓮", "葵", "陽向", "千尋", "湊", "結衣", "悠真",
    "咲良", "大地", "美月", "颯太", "琴音", "拓海", "紗英", "和樹",
    "澪", "翔", "柚希", "直人", "小春", "小夜", "彰", "真昼",
]

DICTIONARY_TABLES = [
    "dictionary_entries",
    "dictionary_frequency",
    "dictionary_pitch",
    "dictionaries",
]

# Where a Noto CJK face is likely to be. A drawn cover carries a Japanese
# title, and a Latin-only fallback would draw a row of empty boxes.
FONT_CANDIDATES = [
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
]

COVER_SIZE = (600, 900)


def backup(src: Path, dst: Path) -> None:
    """Copy through SQLite rather than the filesystem: the live database is
    being written to, and a WAL mid-write copies as a torn file."""
    dst.unlink(missing_ok=True)
    for extra in ("-wal", "-shm"):
        Path(str(dst) + extra).unlink(missing_ok=True)
    source = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
    target = sqlite3.connect(dst)
    with target:
        source.backup(target)
    source.close()
    target.close()


def line_text(rng: random.Random, chars: int) -> str:
    """A line of exactly `chars` characters, so `lines.chars` still holds."""
    if chars <= 0:
        return ""
    out = ""
    while len(out) < chars:
        out += SENTENCES[rng.randrange(len(SENTENCES))]
    return out[:chars]


def title(work):
    if work is None:
        return None
    return WORK_TITLES.get(work, work)


def scrub_knowledge(db: sqlite3.Connection, rng: random.Random) -> None:
    # Works keep their titles. The renamed two also lose the window title,
    # which is the game's own executable name.
    for old, new in WORK_TITLES.items():
        db.execute(
            "UPDATE works SET title = ?, vn_window = NULL WHERE title = ?",
            (new, old),
        )

    # Lines. The text, its wrapped form and its ruby go; ts, chars, discarded
    # and the work stay, which is the whole reading history.
    rows = db.execute("SELECT id, chars, work FROM lines").fetchall()
    db.executemany(
        "UPDATE lines SET text = ?, wrapped = NULL, ruby = NULL, work = ? WHERE id = ?",
        [(line_text(rng, chars), title(work), lid) for lid, chars, work in rows],
    )

    # Manual sessions carry the pasted text of a book session.
    rows = db.execute("SELECT id, chars, work, content FROM manual_sessions").fetchall()
    db.executemany(
        "UPDATE manual_sessions SET content = ?, note = NULL, url = NULL, work = ? WHERE id = ?",
        [
            (line_text(rng, chars) if content else None, title(work), sid)
            for sid, chars, work, content in rows
        ],
    )

    # books.text is a whole flattened epub. Every position is a byte offset into
    # it, so the replacement is generated to length and the offset clamped.
    for work, body_chars, body_start, position in db.execute(
        "SELECT work, body_chars, body_start, position FROM books"
    ).fetchall():
        text = line_text(rng, (body_start or 0) + (body_chars or 0))
        db.execute(
            "UPDATE books SET work = ?, text = ?, text_bytes = ?, position = ? WHERE work = ?",
            (
                title(work),
                text,
                len(text.encode()),
                min(position or 0, len(text.encode())),
                work,
            ),
        )

    # The remaining tables keep their real words and only follow the rename.
    for table in ("lookups", "work_terms", "work_script_terms", "work_scripts"):
        for old, new in WORK_TITLES.items():
            db.execute(f"UPDATE {table} SET work = ? WHERE work = ?", (new, old))

    # A renamed work's cast is replaced: the real names would say which work it
    # is. There are more invented names than any one work has rows, so no two
    # collapse onto the same (work, name) key.
    for old, new in WORK_TITLES.items():
        rows = db.execute(
            "SELECT name FROM work_names WHERE work = ? ORDER BY name", (old,)
        ).fetchall()
        db.execute("DELETE FROM work_names WHERE work = ?", (old,))
        db.executemany(
            "INSERT OR IGNORE INTO work_names (work, name, source) VALUES (?, ?, 'demo')",
            [(new, NAMES[i % len(NAMES)]) for i in range(len(rows))],
        )

    for table in DICTIONARY_TABLES:
        db.execute(f"DELETE FROM {table}")


def find_font() -> str:
    for path in FONT_CANDIDATES:
        if Path(path).exists():
            return path
    sys.exit("no Noto CJK font found — a drawn cover would be a row of boxes")


def draw_cover(name: str, path: Path) -> None:
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        sys.exit("Pillow is needed to draw a cover: pip install pillow")
    import colorsys

    font_path = find_font()
    width, height = COVER_SIZE
    hue = hashlib.sha256(name.encode()).digest()[0] / 255.0

    image = Image.new("RGB", (width, height))
    draw = ImageDraw.Draw(image)
    top = colorsys.hsv_to_rgb(hue, 0.45, 0.55)
    bottom = colorsys.hsv_to_rgb((hue + 0.08) % 1.0, 0.55, 0.22)
    for y in range(height):
        t = y / height
        draw.line(
            [(0, y), (width, y)],
            fill=tuple(int(255 * (top[i] * (1 - t) + bottom[i] * t)) for i in range(3)),
        )
    draw.rectangle([40, 40, width - 40, height - 40], outline=(255, 255, 255, 60))

    size = 58
    font = ImageFont.truetype(font_path, size)
    per_line = max(1, (width - 160) // size)
    words = name.split(" ")
    if len(words) > 1 and max(len(w) for w in words) <= per_line:
        lines, current = [], ""
        for word in words:
            candidate = f"{current} {word}".strip()
            if len(candidate) <= per_line:
                current = candidate
            else:
                lines.append(current)
                current = word
        lines.append(current)
    else:
        lines = [name[i : i + per_line] for i in range(0, len(name), per_line)]

    y = (height - len(lines) * (size + 16)) // 2
    for line in lines:
        box = draw.textbbox((0, 0), line, font=font)
        draw.text(
            ((width - (box[2] - box[0])) // 2, y), line, font=font, fill=(255, 255, 255)
        )
        y += size + 16

    image.save(path, quality=88)


def make_covers(db: sqlite3.Connection, live_covers: Path, out: Path) -> None:
    """Copy each work's real cover; draw one for the works that were renamed.

    A renamed work cannot keep its art, and there is none to copy for the name
    it now has, so those two are drawn from the title.
    """
    out.mkdir(parents=True, exist_ok=True)
    drawn = set(WORK_TITLES.values())
    count = 0

    for name, cover in db.execute(
        "SELECT title, cover_path FROM works WHERE cover_path IS NOT NULL"
    ).fetchall():
        source = live_covers / cover
        if name in drawn or not source.exists():
            draw_cover(name, out / cover)
            count += 1
        else:
            shutil.copy2(source, out / cover)

    print(f"covers: {len(list(out.glob('*.jpg')))} ({count} drawn)")


def scrub_stats(db: sqlite3.Connection) -> None:
    current = db.execute(
        "SELECT value FROM settings WHERE key = 'current_work'"
    ).fetchone()
    if current:
        db.execute(
            "UPDATE settings SET value = ? WHERE key = 'current_work'",
            (title(current[0]),),
        )
    # The machine the reading happened on: a window title, an AnkiConnect
    # address, a WebSocket the demo has no logger for.
    db.execute(
        "UPDATE settings SET value = '' "
        "WHERE key IN ('vn_window', 'anki_source', 'line_source_ws_url')"
    )
    db.execute("DELETE FROM settings WHERE key = 'vn_logger_heartbeat'")
    # The renamed works' covers are drawn, so their VNDB ids would only invite a
    # re-fetch of the art they replaced.
    db.execute("DELETE FROM work_covers")


def verify(out: Path, live_knowledge: Path) -> None:
    """Fail rather than ship a database still holding a script."""
    live = sqlite3.connect(f"file:{live_knowledge}?mode=ro", uri=True)
    demo = sqlite3.connect(out / "knowledge.db")

    kept = {t for (t,) in demo.execute("SELECT title FROM works")} & set(WORK_TITLES)
    if kept:
        sys.exit(f"renamed works kept their titles: {sorted(kept)}")

    sample = {
        t
        for (t,) in live.execute(
            "SELECT text FROM lines WHERE text IS NOT NULL ORDER BY id LIMIT 3000"
        )
    }
    hits = sum(
        1
        for (t,) in demo.execute("SELECT text FROM lines WHERE text IS NOT NULL")
        if t in sample
    )
    if hits:
        sys.exit(f"{hits} original lines survived")

    live_books = {t for (t,) in live.execute("SELECT text FROM books")}
    if any(t in live_books for (t,) in demo.execute("SELECT text FROM books")):
        sys.exit("a book's text survived")

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
    make_covers(knowledge, args.live_dir / "covers", args.out / "covers")
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
