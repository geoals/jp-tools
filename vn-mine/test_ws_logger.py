#!/usr/bin/env python3
"""Tests for vn-ws-logger's clean_line() — the Dohna Dohna dialogue extractor.

Fixtures are real captures from a reading session (opening, an H-scene skipped
at speed, combat, and menu navigation) with the hook HS932#-C@289F60:main.bin.
Run: python3 vn-mine/test_ws_logger.py
"""
import importlib.util
import os
import sys
import types
import unittest

# The module imports websockets (only used by the async pump); stub it so the
# pure-function tests run with no third-party deps installed.
sys.modules.setdefault("websockets", types.ModuleType("websockets"))

_path = os.path.join(os.path.dirname(__file__), "vn-ws-logger.py")
_spec = importlib.util.spec_from_file_location("vn_ws_logger", _path)
wl = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(wl)

# The two literal markers the engine emits verbatim ahead of each run of text.
M = r"\$\{[^\}]+\}"  # heads a dialogue run
U = r"([^\[\]]+?)+|\[[^\]]+?\]"  # heads a UI/animation/widget run


class CleanLine(unittest.TestCase):
    def test_strips_markup_and_speaker(self):
        # A named line: markers gone, 【speaker】 gone, just the dialogue left.
        raw = f"{M}【ポルノ】{M}「つんつつん」"
        self.assertEqual(wl.clean_line(raw), "「つんつつん」")

    def test_unnamed_line(self):
        raw = f"{M}「ふひゃあんっ！！」"
        self.assertEqual(wl.clean_line(raw), "「ふひゃあんっ！！」")

    def test_multi_marker_line_joined(self):
        # Real opening line, split across markers at every soft break.
        raw = (
            f"{M}【＊】{M}『除幕式が行われた萬広場。"
            f"{M}　ここは初代萬様が好んで散策された地として"
            f"{M}　知られております』"
        )
        self.assertEqual(
            wl.clean_line(raw),
            "『除幕式が行われた萬広場。　ここは初代萬様が好んで散策された地として　知られております』",
        )

    def test_pure_ui_dropped(self):
        raw = (
            f"{U}[X:-256 1280 EaseInOutExp|Time:300]{U}Section:MoneyView"
            f"[Number:10000|Time:150]Button\\dText2Button\\dルートパーツ"
        )
        self.assertIsNone(wl.clean_line(raw))

    def test_widget_soup_dropped(self):
        raw = (
            "Button\\dText2Button\\dルートパーツButton\\dClipperButton\\d"
            "Button1Button\\dBaseButton\\dTextAreaButton\\dText1"
        )
        self.assertIsNone(wl.clean_line(raw))

    def test_combat_labels_dropped(self):
        # enemy-name / armour-tier soup that leaked in from the battle screen
        raw = f"\\d+猿0\\d+猿0\\d+軽装0\\d+軽装0{U}Section:SceneBattle[X:-640 0 EaseInOutExp]"
        self.assertIsNone(wl.clean_line(raw))

    def test_ui_then_dialogue_keeps_only_dialogue(self):
        # A capture that fuses a nameplate animation with the line it heralds.
        raw = (
            f"{U}Section:AdvNamePlate[Time:150|X:0 -30 EaseOutQuad Rel|Alpha: 255 0]"
            f"{U}Section:AdvEventCg [Time:250|Alpha:0 255]"
            f"{M}【＊＊】{M}『――はい。{M}　現場より素晴らしい瞬間の中継でした』"
        )
        got = wl.clean_line(raw)
        self.assertEqual(got, "『――はい。　現場より素晴らしい瞬間の中継でした』")
        for junk in ("Section:", "Ease", "$", "【", "["):
            self.assertNotIn(junk, got)

    def test_skip_through_dropped(self):
        # Holding skip fuses a crowd of 【speaker】-headed lines into one capture.
        raw = M + M.join(
            f"【クマ】{M}「ん…{i}…」{M}地の文が続く。{M}【ポルノ】{M}「はぁ…」"
            for i in range(3)
        )
        self.assertGreaterEqual(len(wl._SPEAKER.findall(raw)), wl.MAX_SPEAKER_TAGS)
        self.assertIsNone(wl.clean_line(raw))

    def test_four_speakers_still_kept(self):
        # Just below the skip threshold — a brisk exchange is still real reading.
        raw = "".join(f"{M}【{n}】{M}「x{n}」" for n in "アイウエ")
        self.assertEqual(len(wl._SPEAKER.findall(raw)), 4)
        self.assertIsNotNone(wl.clean_line(raw))

    def test_other_game_passthrough(self):
        # No Dohna markers: some other VN's hook — leave it exactly as-is.
        self.assertEqual(wl.clean_line("「こんにちは」"), "「こんにちは」")
        self.assertEqual(wl.clean_line("　あれは……夢だったのか。"), "　あれは……夢だったのか。")

    def test_runaway_capture_dropped(self):
        self.assertIsNone(wl.clean_line("あ" * (wl.MAX_READING_CHARS + 1)))

    def test_non_japanese_dropped(self):
        self.assertIsNone(wl.clean_line("OK Cancel Button"))
        self.assertIsNone(wl.clean_line(f"{U}[Alpha:255 0|Time:1000]"))


class RealLogInvariants(unittest.TestCase):
    """If the session log is still on tmpfs, assert no junk survives on any line."""

    def setUp(self):
        self.log = os.path.expanduser(
            os.environ.get("VN_RUNDIR", f"/run/user/{os.getuid()}/vn-mine")
            + "/lines.log"
        )
        if not os.path.exists(self.log) or os.path.getsize(self.log) == 0:
            self.skipTest("no session lines.log present")

    def test_no_markup_survives(self):
        kept = 0
        with open(self.log, encoding="utf-8") as f:
            lines = f.readlines()
        for line in lines:
            raw = line.rstrip("\n").split("\t", 1)[-1]
            out = wl.clean_line(wl.normalize(raw))
            if out is None:
                continue
            kept += 1
            for junk in ("Section:", "Button\\d", "${", "\\$\\{", "【"):
                self.assertNotIn(junk, out, f"leaked {junk!r} from: {raw[:80]}")
        self.assertGreater(kept, 0, "expected at least some dialogue in the log")


if __name__ == "__main__":
    unittest.main(verbosity=2)
