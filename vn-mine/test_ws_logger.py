#!/usr/bin/env python3
"""Tests for vn-ws-logger's clean_line() — the Dohna Dohna dialogue extractor.

Fixtures are real captures from a reading session (opening, an H-scene skipped
at speed, combat, and menu navigation) with the hook HS932#-C@289F60:main.bin.
Run: python3 vn-mine/test_ws_logger.py
"""
import asyncio
import importlib.util
import io
import os
import sqlite3
import sys
import tempfile
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

    def test_soft_break_becomes_newline(self):
        # Manosaba emits its soft breaks as literal markup. Real capture.
        raw = "（もしそんな魔法が魔女化によって強化され、<br>　暴走したのだとしたら……）"
        self.assertEqual(
            wl.clean_line(raw),
            "（もしそんな魔法が魔女化によって強化され、\n　暴走したのだとしたら……）",
        )

    def test_soft_break_variants(self):
        self.assertEqual(wl.clean_line("あ<br/>い"), "あ\nい")
        self.assertEqual(wl.clean_line("あ<BR />い"), "あ\nい")

    def test_script_escapes_decoded(self):
        self.assertEqual(wl.clean_line("\\cd「……は？」"), "「……は？」")
        self.assertEqual(wl.clean_line("\\cdあれは\\n夢だった。"), "あれは\n夢だった。")
        self.assertEqual(wl.clean_line("\\cd「誓えるわよね？」\\@"), "「誓えるわよね？」")

    def test_colour_codes_stripped_line_kept(self):
        # Real capture: a grey narration line, colour set twice ahead of it.
        raw = (
            "\\cd\\cd0xff898989;\\c0xff898989;自分の知識の中にあるアンドロイドの情報と、"
            "\\nそれほどズレのない回答だった。"
        )
        self.assertEqual(
            wl.clean_line(raw),
            "自分の知識の中にあるアンドロイドの情報と、\nそれほどズレのない回答だった。",
        )
        self.assertEqual(wl.clean_line("\\cd0xff898989;\\cd渇いた音が鳴った。"), "渇いた音が鳴った。")

    def test_bracket_furigana_becomes_ruby(self):
        raw = "その[眸/ひとみ]が、おれを見つめている。"
        text, ruby = wl.split_ruby(wl.clean_line(raw))
        self.assertEqual(text, "その眸が、おれを見つめている。")
        self.assertEqual(ruby, [[2, 1, "ひとみ"]])

    def test_other_backslashes_still_drop(self):
        self.assertIsNone(wl.clean_line("Button\\dText2Buttonルートパーツ"))

    def test_rich_text_tags_stripped_content_kept(self):
        # Real capture. The tag goes, the text it wrapped stays.
        raw = "だから……<color=#9c8eff>b</color>！ アリサ！"
        self.assertEqual(wl.clean_line(raw), "だから……b！ アリサ！")

    def test_rich_text_variants(self):
        self.assertEqual(wl.clean_line("<b>強調</b>する"), "強調する")
        self.assertEqual(wl.clean_line("<size=120%>大</size>きい"), "大きい")
        self.assertEqual(wl.clean_line("<i>斜</i>め<nobr>です</nobr>"), "斜めです")

    def test_ascii_angle_brackets_survive(self):
        # Not a tag: emoticons put ASCII angle brackets in real dialogue.
        self.assertEqual(wl.clean_line("そうか<(_ _)>ごめん"), "そうか<(_ _)>ごめん")

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

    def test_script_control_codes_are_stripped_not_dropped(self):
        # Subahibi heads a narration line with \x05 and puts \x04 mid-clause.
        # The line is real reading and stays; the markup is the VN's and goes,
        # or Sudachi analyses it and "e"/"d" turn up in the vocabulary ledger.
        self.assertEqual(wl.clean_line("\x05御霊祭だかなんだか"), "御霊祭だかなんだか")
        self.assertEqual(
            wl.clean_line("「うん？\x04\u3000君のお兄さん」"), "「うん？\u3000君のお兄さん」"
        )

    def test_control_codes_do_not_change_the_character_count(self):
        # Both counters are allowlists, so this has to be a no-op either way —
        # stripping markup must never move a reading statistic.
        self.assertEqual(
            len(wl.NOT_COUNTED.sub("", "\x05御霊祭")),
            len(wl.NOT_COUNTED.sub("", "御霊祭")),
        )

    def test_runaway_capture_dropped(self):
        self.assertIsNone(wl.clean_line("あ" * (wl.MAX_READING_CHARS + 1)))

    def test_emphasis_brackets_survive_a_markerless_line(self):
        # Not every 【】 is a speaker. A game that carries none of Dohna Dohna's
        # markers passes through whole, emphasis included — stripping it here
        # would remove exactly the term the writer was pointing at.
        line = "私の魔法は【幻視】なの。"
        self.assertEqual(wl.clean_line(wl.normalize(line)), line)

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
            for junk in ("Section:", "Button\\d", "${", "\\$\\{"):
                self.assertNotIn(junk, out, f"leaked {junk!r} from: {raw[:80]}")
            # 【】 is a *speaker tag* only in the script-layer captures that
            # carry Dohna Dohna's markers, and only there is it stripped. In a
            # marker-less capture it is ordinary emphasis — 魔法少女ノ魔女裁判
            # writes 私の魔法は【幻視】なの — so asserting it never survives
            # would be asserting that the word it marks gets eaten.
            if len(wl._SEGMENT.split(raw)) > 1:
                self.assertNotIn("【", out, f"speaker tag leaked from: {raw[:80]}")
        self.assertGreater(kept, 0, "expected at least some dialogue in the log")


def make_databases(tmp, paused=None):
    """Both databases as the migrations leave them, with only what the sink
    needs — built from `REQUIRED`, so a column added there without one added
    here fails loudly rather than at the first insert of a session."""
    lines = ", ".join(sorted(wl.REQUIRED["lines"]))
    knowledge = os.path.join(tmp, "knowledge.db")
    db = sqlite3.connect(knowledge, isolation_level=None)
    db.execute(f"CREATE TABLE lines (id INTEGER PRIMARY KEY, {lines})")
    db.close()

    stats = os.path.join(tmp, "read-stats.db")
    db = sqlite3.connect(stats, isolation_level=None)
    db.execute("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
    if paused is not None:
        db.execute("INSERT INTO settings VALUES ('capture_paused', ?)", (paused,))
    db.close()
    return knowledge, stats


class CapturePausedFlag(unittest.TestCase):
    """The one contract shared with read-stats: it writes the flag, this reads
    it. A regression here doesn't fail loudly — it silently keeps capturing."""

    def sink(self, value):
        tmp = tempfile.mkdtemp()
        knowledge, stats = make_databases(tmp, paused=value)
        self.addCleanup(os.environ.pop, "JP_TOOLS_STATS_DISABLE", None)
        os.environ.pop("JP_TOOLS_STATS_DISABLE", None)
        old = (wl.KNOWLEDGE_DB, wl.STATS_DB)
        wl.KNOWLEDGE_DB, wl.STATS_DB = knowledge, stats
        self.addCleanup(lambda: setattr(wl, "KNOWLEDGE_DB", old[0]))
        self.addCleanup(lambda: setattr(wl, "STATS_DB", old[1]))
        return wl.StatsSink()

    def test_paused_when_flag_is_one(self):
        self.assertTrue(self.sink("1").capture_paused())

    def test_not_paused_when_flag_is_zero(self):
        self.assertFalse(self.sink("0").capture_paused())

    def test_not_paused_when_row_absent(self):
        self.assertFalse(self.sink(None).capture_paused())

    def test_fails_open_with_no_database(self):
        sink = self.sink("1")
        sink.db = None
        self.assertFalse(
            sink.capture_paused(), "an unreadable flag must keep capturing"
        )


class SchemaIsNotOurs(unittest.TestCase):
    """The schema belongs to jp-core's migrations and read-stats'. This daemon
    checks for it and waits; it must never create or alter a table, or the copy
    it creates becomes the one that goes stale."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        os.environ.pop("JP_TOOLS_STATS_DISABLE", None)
        self.addCleanup(os.environ.pop, "JP_TOOLS_STATS_DISABLE", None)
        old = (wl.KNOWLEDGE_DB, wl.STATS_DB)
        wl.KNOWLEDGE_DB = os.path.join(self.tmp, "knowledge.db")
        wl.STATS_DB = os.path.join(self.tmp, "read-stats.db")
        self.addCleanup(lambda: setattr(wl, "KNOWLEDGE_DB", old[0]))
        self.addCleanup(lambda: setattr(wl, "STATS_DB", old[1]))

    def test_no_database_creates_none(self):
        sink = wl.StatsSink()
        self.assertIsNone(sink.db)
        self.assertFalse(os.path.exists(wl.KNOWLEDGE_DB), "created a database")

    def test_a_missing_column_is_refused(self):
        db = sqlite3.connect(wl.KNOWLEDGE_DB, isolation_level=None)
        db.execute("CREATE TABLE lines (id INTEGER PRIMARY KEY, ts REAL, chars INTEGER)")
        db.close()
        db = sqlite3.connect(wl.STATS_DB, isolation_level=None)
        db.execute("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        db.close()
        self.assertIsNone(wl.StatsSink().db, "wrote against a schema it cannot satisfy")

    def test_it_opens_once_the_migrations_have_run(self):
        sink = wl.StatsSink()
        self.assertIsNone(sink.db)
        make_databases(self.tmp)
        sink._next_try = 0
        self.assertTrue(sink.ready(), "a sink that gave up loses the whole sitting")
        sink.add(1.0, "テスト")
        self.assertEqual(
            sink.db.execute("SELECT text FROM lines").fetchall(), [("テスト",)]
        )


class SplitCaptureRejoined(unittest.TestCase):
    """Textractor splitting one script line at its own \\n, as it really
    arrived: two captures 30ms apart, the second opening on the break."""

    HALVES = [
        "\\cd「今僕は、その理由を聞かせてもらおうとしている。",
        "\\n後学のためにね」",
    ]
    WHOLE = "「今僕は、その理由を聞かせてもらおうとしている。\n後学のためにね」"

    def sink(self):
        tmp = tempfile.mkdtemp()
        knowledge, stats = make_databases(tmp)
        self.addCleanup(os.environ.pop, "JP_TOOLS_STATS_DISABLE", None)
        os.environ.pop("JP_TOOLS_STATS_DISABLE", None)
        old = (wl.KNOWLEDGE_DB, wl.STATS_DB)
        wl.KNOWLEDGE_DB, wl.STATS_DB = knowledge, stats
        self.addCleanup(lambda: setattr(wl, "KNOWLEDGE_DB", old[0]))
        self.addCleanup(lambda: setattr(wl, "STATS_DB", old[1]))
        return wl.StatsSink()

    def drain(self, captures):
        sink = self.sink()

        async def ws():
            for c in captures:
                yield c

        asyncio.run(wl.read_lines(ws(), io.StringIO(), sink, (None, [])))
        return sink.db.execute(
            "SELECT text, discarded FROM lines ORDER BY id"
        ).fetchall()

    def test_the_feed_ends_on_the_whole_line(self):
        rows = self.drain(self.HALVES)
        self.assertEqual(
            [r[0] for r in rows if not r[1]],
            [self.WHOLE],
            "the reader must see one line, not a half that vanishes",
        )

    def test_the_half_is_discarded_not_deleted(self):
        rows = self.drain(self.HALVES)
        self.assertEqual([r[1] for r in rows], [1, 0])

    def test_a_continuation_alone_still_logs(self):
        # No previous line to attach to — after a reconnect, or with the first
        # half dropped. Better a fragment than a lost line.
        rows = self.drain(self.HALVES[1:])
        self.assertEqual(rows, [("後学のためにね」", 0)])

    def test_a_cleared_textbox_is_not_a_continuation(self):
        self.assertFalse(wl.continues_previous("\\cd\\n後学のためにね」"))


class SplitRuby(unittest.TestCase):
    def test_engine_form(self):
        # Real capture. The reading leaves the line and travels beside it.
        text, ruby = wl.split_ruby(
            "彼女の悪戯が<ruby=\"おおごと\">大事</ruby>になってしまった……。"
        )
        self.assertEqual(text, "彼女の悪戯が大事になってしまった……。")
        self.assertEqual(ruby, [[6, 2, "おおごと"]])

    def test_html_form_with_fallback_parens(self):
        text, ruby = wl.split_ruby("<ruby>節<rp>(</rp><rt>ふし</rt><rp>)</rp></ruby>穴")
        self.assertEqual(text, "節穴")
        self.assertEqual(ruby, [[0, 1, "ふし"]])

    def test_several_annotations_offset_from_stripped_text(self):
        text, ruby = wl.split_ruby("<ruby=\"あ\">亜</ruby>と<ruby=\"い\">伊</ruby>")
        self.assertEqual(text, "亜と伊")
        self.assertEqual(ruby, [[0, 1, "あ"], [2, 1, "い"]])

    def test_unclosed_tag_drops_its_reading(self):
        # Furigana is a gloss on the spelling; a broken tag must not leak it
        # into the line, where it would be counted and tokenized.
        self.assertEqual(wl.split_ruby("大事<rt>おおごと"), ("大事", []))

    def test_plain_line_untouched(self):
        self.assertEqual(wl.split_ruby("ふつうの行"), ("ふつうの行", []))

    def test_offsets_are_utf16(self):
        # 𠮟 is a surrogate pair in UTF-16, which is what the overlay indexes
        # in — a codepoint offset would place the furigana one short.
        text, ruby = wl.split_ruby("𠮟<ruby=\"しか\">叱</ruby>")
        self.assertEqual(text, "𠮟叱")
        self.assertEqual(ruby, [[2, 1, "しか"]])


class StripSpeakerTest(unittest.TestCase):
    def test_name_field_removed(self):
        self.assertEqual(wl.strip_speaker("恵輔「ッ！」"), "「ッ！」")
        self.assertEqual(wl.strip_speaker("？？？「恵ちゃん！」"), "「恵ちゃん！」")

    def test_furigana_on_the_name_goes_with_it(self):
        self.assertEqual(wl.strip_speaker("<ruby=けいすけ>恵輔</ruby>「そうか」"), "「そうか」")

    def test_quote_inside_a_line_is_not_a_name(self):
        self.assertEqual(wl.strip_speaker("俺は「バカ」と呼ばれた。"), "俺は「バカ」と呼ばれた。")
        self.assertEqual(wl.strip_speaker("そう言って笑った。「またね」"), "そう言って笑った。「またね」")

    def test_two_fused_lines_are_not_a_name(self):
        # A capture that caught the tail of the previous line. Real log entry.
        line = "どうだ？」「どうだって言われても……」"
        self.assertEqual(wl.strip_speaker(line), line)

    def test_narration_and_unnamed_dialogue_untouched(self):
        self.assertEqual(wl.strip_speaker("「そうだね」"), "「そうだね」")
        self.assertEqual(wl.strip_speaker("心配そうに叶が呼ぶ。"), "心配そうに叶が呼ぶ。")


class CollapseRepeatsTest(unittest.TestCase):
    """A hook that emits every character four times over."""

    def test_plain_line(self):
        raw = "心心心心配配配配そそそそううううにににに呼呼呼呼ぶぶぶぶ。。。。"
        self.assertEqual(wl.collapse_repeats(raw), "心配そうに呼ぶ。")

    def test_genuine_repeat_survives(self):
        # What Textractor's own filter gets wrong: it collapses the run to one.
        raw = "だだだだかかかかららららああああああああ。。。。"
        self.assertEqual(wl.collapse_repeats(raw), "だからああ。")

    def test_inlined_furigana_becomes_a_ruby_tag(self):
        raw = "俺俺俺俺はははは瞠瞠どどううももくく瞠瞠どどううももくく目目目目すすすするるるる。。。。"
        text, ruby = wl.split_ruby(wl.clean_line(raw))
        self.assertEqual(text, "俺は瞠目する。")
        self.assertEqual(ruby, [[2, 2, "どうもく"]])

    def test_reading_stops_at_its_own_word(self):
        # 帆刈(ほかり)叶(かなえ): the second name must not be pulled under the first
        # reading, and 刈 must be.
        raw = "帆帆ほほかかりり帆帆ほほかかりり刈刈刈刈叶叶かかななええ叶叶かかななええだだだだ。。。。"
        text, ruby = wl.split_ruby(wl.clean_line(raw))
        self.assertEqual(text, "帆刈叶だ。")
        self.assertEqual(ruby, [[0, 2, "ほかり"], [2, 1, "かなえ"]])

    def test_speaker_name_is_a_fragment_not_a_reading(self):
        raw = "恵恵輔輔恵恵輔輔「「「「ッッッッ！！！！」」」」"
        text, ruby = wl.split_ruby(wl.clean_line(raw))
        self.assertEqual(text, "「ッ！」")  # the name field is stripped after
        self.assertEqual(ruby, [])

    def test_kana_tailed_name_keeps_its_kana(self):
        # 恵ちゃん is one kanji then kana, the shape furigana also has. The line
        # -initial fragment ahead of an opening quote is the speaker field.
        raw = "恵恵ちちゃゃんん恵恵ちちゃゃんん「「「「そそそそうううう？？？？」」」」"
        text, ruby = wl.split_ruby(wl.clean_line(raw))
        self.assertEqual(text, "「そう？」")  # the name field is stripped after
        self.assertEqual(ruby, [])

    def test_other_games_untouched(self):
        self.assertIsNone(wl.collapse_repeats("これは普通の行です。"))
        self.assertIsNone(wl.collapse_repeats("そんなーーーー！"))
        self.assertEqual(wl.clean_line("これは普通の行です。"), "これは普通の行です。")

    def test_quadrupled_line_survives_the_length_guard(self):
        # 400 characters of hook for a 100-character line, over MAX_READING_CHARS.
        raw = "".join(c * 4 for c in "あいうえお" * 20)
        self.assertEqual(wl.clean_line(raw), "あいうえお" * 20)


if __name__ == "__main__":
    unittest.main(verbosity=2)
