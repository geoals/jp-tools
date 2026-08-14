#!/usr/bin/env python3
"""Dump the script text of a CatSystem2 visual novel to one line per file.

Reads scene.int (a KIF archive of compiled CatScene scripts) and writes the
dialogue and narration, with the engine's markup resolved.

    ./cs2-script.py ~/Games/<game>/drive_c/<dir> out.txt

Needs pycryptodome.
"""
import argparse
import os
import re
import struct
import sys
import zlib

from Crypto.Cipher import Blowfish


class MersenneTwister:
    """CatSystem2 seeds with the old Knuth 69069 loop, not MT19937's 0x6c078965."""

    N, M = 624, 397

    def __init__(self, seed):
        self.mt = [0] * self.N
        self.mti = self.N
        self.srand(seed)

    def srand(self, seed):
        seed &= 0xFFFFFFFF
        for i in range(self.N):
            upper = seed & 0xFFFF0000
            seed = (69069 * seed + 1) & 0xFFFFFFFF
            self.mt[i] = upper | ((seed & 0xFFFF0000) >> 16)
            seed = (69069 * seed + 1) & 0xFFFFFFFF
        self.mti = self.N

    def rand(self):
        if self.mti >= self.N:
            mt, mag01 = self.mt, (0, 0x9908B0DF)
            for kk in range(self.N - self.M):
                y = (mt[kk] & 0x80000000) | (mt[kk + 1] & 0x7FFFFFFF)
                mt[kk] = mt[kk + self.M] ^ (y >> 1) ^ mag01[y & 1]
            for kk in range(self.N - self.M, self.N - 1):
                y = (mt[kk] & 0x80000000) | (mt[kk + 1] & 0x7FFFFFFF)
                mt[kk] = mt[kk + self.M - self.N] ^ (y >> 1) ^ mag01[y & 1]
            y = (mt[self.N - 1] & 0x80000000) | (mt[0] & 0x7FFFFFFF)
            mt[self.N - 1] = mt[self.M - 1] ^ (y >> 1) ^ mag01[y & 1]
            self.mti = 0
        y = self.mt[self.mti]
        self.mti += 1
        y ^= y >> 11
        y ^= (y << 7) & 0x9D2C5680
        y ^= (y << 15) & 0xEFC60000
        y &= 0xFFFFFFFF
        return y ^ (y >> 18)


def _swap_words(b):
    return b"".join(b[i:i + 4][::-1] for i in range(0, len(b), 4))


def _decipher(cipher, data):
    """File data is word-swapped around Blowfish; the directory pair is not."""
    n = len(data) // 8 * 8
    return _swap_words(cipher.decrypt(_swap_words(data[:n]))) + data[n:]


def read_archive(path):
    """Yield the decrypted contents of every entry in a KIF .int archive.

    Entry names are skipped: deciphering them needs a key from the game exe,
    which 2020-era builds no longer carry, and only the names depend on it.
    """
    data = open(path, "rb").read()
    if data[:4] != b"KIF\0":
        sys.exit(f"{path}: not a KIF archive")
    count = struct.unpack_from("<i", data, 4)[0]
    if data[8:20] != b"__key__.dat\0":
        off = 8
        for _ in range(count):
            o, s = struct.unpack_from("<II", data, off + 0x40)
            yield data[o:o + s]
            off += 0x48
        return

    seed = struct.unpack_from("<I", data, 8 + 0x44)[0]
    twister = MersenneTwister(seed)
    cipher = Blowfish.new(struct.pack("<I", twister.rand()), Blowfish.MODE_ECB)
    off = 8
    for i in range(1, count):
        off += 0x48
        o, s = struct.unpack_from("<II", data, off + 0x40)
        o, s = struct.unpack(">II", cipher.decrypt(
            struct.pack(">II", (o + i) & 0xFFFFFFFF, s)))
        yield _decipher(cipher, data[o:o + s])


def inflate_scene(blob):
    if blob[:8] != b"CatScene":
        return None
    compressed = struct.unpack_from("<I", blob, 8)[0]
    return zlib.decompress(blob[16:16 + compressed])


TEXT, NAME, COMMAND = 0x20, 0x21, 0x30

RUBY = re.compile(r"\[([^\[\]/]*)/[^\[\]]*\]")
# \n wraps, \@ waits for a click, \fll picks a font: display, not text.
DISPLAY_CODE = re.compile(r"\\[a-zA-Z@]*")


def clean(text):
    """Resolve ruby to its base text and drop the display codes.

    The base is the spelling the tokenizer must see: the reading beside it is
    often a gloss rather than a reading (ＳＵＶ over the words it stands for).
    """
    text = RUBY.sub(r"\1", text)
    return DISPLAY_CODE.sub("", text).strip()


SCENE_LABEL = re.compile(r"\bc(\d\d)_")


def scene_part(script):
    """Which part of the game a scene belongs to, from the labels it jumps to.

    CatSystem2 gives a scene no name of its own that survives extraction — the
    archive's filenames need a key the game exe stopped carrying — but the jump
    targets in its command lines are named (`c01_04`), and a scene's jumps stay
    inside its own part except at a boundary. Majority vote, so one forward
    jump into the next part does not relabel the scene making it.

    Returns None where a scene names no target: menus, system scenes, and the
    branch stubs that only pick a route.
    """
    base = 0x10 + struct.unpack_from("<I", script, 0x0C)[0]
    if base >= len(script):
        return None
    seen = {}
    for record in script[base:].split(b"\0"):
        if len(record) > 2 and record[1] == COMMAND:
            try:
                text = record[2:].decode("cp932")
            except UnicodeDecodeError:
                continue
            for part in SCENE_LABEL.findall(text):
                seen[part] = seen.get(part, 0) + 1
    return max(seen, key=seen.get) if seen else None


def scene_lines(script, keep_names):
    base = 0x10 + struct.unpack_from("<I", script, 0x0C)[0]
    if base >= len(script):
        return
    for record in script[base:].split(b"\0"):
        if len(record) <= 2 or record[1] not in (TEXT, NAME):
            continue
        if record[1] == NAME and not keep_names:
            continue
        try:
            line = clean(record[2:].decode("cp932"))
        except UnicodeDecodeError:
            continue
        if line:
            yield line


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("game_dir", help="directory holding scene.int")
    ap.add_argument("out", help="output text file")
    ap.add_argument("--names", action="store_true",
                    help="include speaker names as their own lines")
    ap.add_argument("--split", action="store_true",
                    help="one file per part of the game (<out>.c00.txt, …), "
                         "for a work whose routes are read one at a time")
    args = ap.parse_args()

    archive = os.path.join(args.game_dir, "scene.int")
    if not os.path.exists(archive):
        sys.exit(f"no scene.int in {args.game_dir}")

    scenes = written = 0
    parts = {}
    with open(args.out, "w", encoding="utf-8") as out:
        for blob in read_archive(archive):
            script = inflate_scene(blob)
            if script is None:
                continue
            scenes += 1
            lines = list(scene_lines(script, args.names))
            part = scene_part(script) if args.split else None
            for line in lines:
                out.write(line + "\n")
                written += 1
            if args.split and part and lines:
                parts.setdefault(part, []).extend(lines)
    print(f"{scenes} scenes, {written} lines -> {args.out}")

    stem = args.out[:-4] if args.out.endswith(".txt") else args.out
    for part, lines in sorted(parts.items()):
        path = f"{stem}.c{part}.txt"
        with open(path, "w", encoding="utf-8") as out:
            out.write("\n".join(lines) + "\n")
        print(f"  part c{part}: {len(lines)} lines -> {path}")


if __name__ == "__main__":
    main()
