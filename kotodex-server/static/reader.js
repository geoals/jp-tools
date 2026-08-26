/** The reading view: the live Textractor line feed, sized to sit beside the
 *  running VN.
 *
 *  There is no mine button — Yomitan's addNote goes through the AnkiConnect
 *  proxy, which fires vn-capture.sh once Anki accepts the note.
 *
 *  **Lines are plain text nodes.** Yomitan scans the DOM, so virtualized rows or
 *  per-token spans would break lookups. The word marks work around that rather
 *  than through it: a layer of rectangles behind the text, never markup inside
 *  it. See `paintMarks` at the foot of this file. */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "preact/hooks";
import { html } from "htm/preact";
import { parseMarkdown } from "/shared/markdown.js";
import { streamExplain } from "/shared/explain.js";
import { api } from "./api.js";

const FONT_KEY = "reader-font-px";
const FONT_DEFAULT = 20;
const HIGHLIGHT_KEY = "reader-highlight";
const MARKED_ONLY_KEY = "reader-marked-only";
/** The statuses that get a colour. `known` is sent too, since a span is also
 *  the region a tap judges, but is absent here: the absence of a mark is what
 *  makes the marks readable. Anything unrecognised is ignored rather than
 *  drawn — that is version skew, not a word. */
const PAINTED = ["seen", "new", "unknown"];
/** Where the common-word underline sits and how thick it is. Drawn inside the
 *  mark's rectangle, which sits behind the text, so it reads as an underline
 *  under the glyphs rather than a second bar. */
const UNDERLINE_PX = 3;
/** How far a mark is held back from its word on each side. Japanese sets
 *  without spaces, so two adjacent marked words have touching rects: a mark
 *  that reached *past* its word overlapped its neighbour and the two read as
 *  one bar, corner radius and all. The inset is what makes the boundary
 *  visible. */
const MARK_INSET_PX = 1;
/** Distance from the bottom that still counts as "following along", so
 *  scrolling up to re-read isn't yanked back by the next line. */
const STICK_SLOP_PX = 80;
const TOAST_MS = 6000;
/** Longer than an ordinary toast: this one is the only route back to a cleared
 *  line, and clearing a handful of them is several taps. */
const UNDO_TOAST_MS = 15000;
/** How often the work title and pause state are re-read — fast enough that
 *  switching works on the dashboard lands before the next card is mined. */
const STATE_POLL_MS = 20_000;
/** Lines sent to the model with the one to explain — enough to place a pronoun
 *  or an unstated subject. The server caps it again. */
const EXPLAIN_CONTEXT_LINES = 8;
/** Groups the feed into sessions before `/api/reader/state` answers, so the
 *  first paint isn't one header over everything. */
const DEFAULT_SESSION_GAP_SECS = 600;
/** One backscroll page, matching the server's default so a full page really is
 *  more history rather than one that reads as "nothing left". */
const HISTORY_PAGE = 200;
/** How close to the top of the scroller counts as "reaching for more
 *  history" — close enough that the fetch lands before the reader gets there. */
const HISTORY_TRIGGER_PX = 300;
/** How many pages the automatic top-up may pull while the `◌ marked` filter is
 *  on. Enough to reach a scrollable feed, far short of the whole history. */
const FILTERED_TOPUP_PAGES = 5;

export function Reader() {
  const [lines, setLines] = useState([]);
  const [live, setLive] = useState(false);
  const [capture, setCapture] = useState(null);
  const [state, setState] = useState(null);
  const [clearing, setClearing] = useState(false);
  const [explaining, setExplaining] = useState(false);
  const [explain, setExplain] = useState(null);
  const [toast, setToast] = useState(null);
  const [loadingHistory, setLoadingHistory] = useState(false);
  /** The two ranks at or below which an unknown word is called common and gets
   *  an underline, one per frequency list, tested independently. Held here
   *  rather than applied server-side so changing a setting repaints the feed
   *  already on screen. Both 0 until the settings arrive, which underlines
   *  nothing — the safe way to be wrong for one paint. */
  const [commonRanks, setCommonRanks] = useState(NO_COMMON_RANKS);
  /** Set once a backscroll page comes back empty, to stop the trigger firing.
   *  A ref rather than state: nothing on screen changes when it flips. */
  const historyExhausted = useRef(false);
  const [fontPx, setFontPx] = useState(
    () => Number(localStorage.getItem(FONT_KEY)) || FONT_DEFAULT,
  );
  // Highlighting is on unless turned off. A browser that cannot paint it isn't
  // offered the toggle at all.
  const [highlightOn, setHighlightOn] = useState(
    () => localStorage.getItem(HIGHLIGHT_KEY) !== "off",
  );
  // Off by default and persisted like the font and the tinting, so a reload
  // beside the VN comes back the way it was left.
  const [markedOnly, setMarkedOnly] = useState(
    () => localStorage.getItem(MARKED_ONLY_KEY) === "on",
  );
  /** The ids the filter lets through: every line that has *had* a marked word
   *  since the filter was turned on. Grows only, rebuilt on each turn-on.
   *
   *  Membership rather than a live predicate, or judging the last marked word in
   *  a line would delete that line from under the finger that judged it. The
   *  line staying, unmarked, is also the report. */
  const [keptIds, setKeptIds] = useState(() => new Set());
  const listRef = useRef(null);
  const stick = useRef(true);
  /** line id → the <p> holding it. A Range needs the text node itself, so the
   *  elements have to be reachable outside the render that made them. */
  const lineEls = useRef(new Map());
  /** The layer the marks are drawn into. Outside Preact's reconciliation: it is
   *  decorative and rewritten wholesale, and routing a few hundred positioned
   *  rectangles through the vdom would re-render the feed to move a box. */
  const marksRef = useRef(null);
  /** The line a backscroll is holding still — `{id, offsetTop}` for the topmost
   *  line on screen when the page was requested. See the effect below. */
  const historyAnchor = useRef(null);
  /** The feed's height the last time the bottom was pinned. A reflow moves the
   *  bottom edge without changing any line id, which is what this notices. */
  const pinnedHeight = useRef(0);
  /** Pages the top-up has spent filling the pane while filtering. Reset on each
   *  turn-on, which is a fresh question over whatever has since been read. */
  const filteredTopUps = useRef(0);

  /** The lines actually on screen. `lines` stays the whole feed whatever the
   *  filter does, so a judgement rewrites hidden occurrences too — a mark
   *  reappearing when the filter comes off would read as a failed write.
   *
   *  Everything that measures or hit-tests the text (`paintMarks`,
   *  `spanAtPoint`) takes *this* list: those index into elements, and a hidden
   *  line has none. */
  const visible = markedOnly ? lines.filter((l) => keptIds.has(l.id)) : lines;

  useEffect(() => {
    api("/api/settings")
      .then((s) =>
        setCommonRanks({
          freq: s.reader_common_max_freq_rank || 0,
          bccwj: s.reader_common_max_bccwj_rank || 0,
        }),
      )
      .catch(() => {});
  }, []);

  useEffect(() => {
    // EventSource reconnects on its own and replays from Last-Event-ID, so a
    // backgrounded tab or a slept screen resumes without losing lines.
    const es = new EventSource("/api/lines/stream");
    es.onopen = () => setLive(true);
    es.onerror = () => setLive(false);
    // The badge reports the *capture pipeline*, not this connection — the SSE
    // stream stays healthy while the logger is detached from Textractor or
    // cannot write, which is the outage worth interrupting reading for.
    es.addEventListener("status", (ev) => setCapture(JSON.parse(ev.data)));
    es.onmessage = (ev) => {
      const line = JSON.parse(ev.data);
      setLines((prev) => {
        // ids are monotonic; anything at or below the tail is a reconnect replay.
        const last = prev.length ? prev[prev.length - 1].id : 0;
        if (line.id <= last) return prev;
        return [...prev, line];
      });
    };
    return () => es.close();
  }, []);

  useEffect(() => {
    const load = () =>
      api("/api/reader/state")
        .then(setState)
        .catch(() => setState((s) => s ?? { capture_available: false }));
    load();
    // Polled rather than fetched once: current_work drives the document
    // title that ends up on the card, and a desktop-hotkey pause shows here.
    const t = setInterval(load, STATE_POLL_MS);
    return () => clearInterval(t);
  }, []);

  // A line that arrives (or is scrolled back into) while the filter is on joins
  // it if it has a marked word — otherwise a filtered feed could never grow. It
  // is only ever added: see `keptIds`.
  useEffect(() => {
    if (!markedOnly) return;
    setKeptIds((prev) => {
      const next = new Set(prev);
      for (const l of lines) if (hasMark(l)) next.add(l.id);
      return next.size === prev.size ? prev : next;
    });
  }, [lines, markedOnly]);

  // A feed shorter than its pane fires no scroll event, so the trigger above
  // can never be reached — the case of a sitting that has just started. Pull
  // pages until there is something to scroll, or there is no more history.
  //
  // While filtering this runs on a budget rather than not at all. Off entirely,
  // a filtered feed holding one line could not scroll, and the scroll trigger is
  // the only other way to ask for history, so the view was stuck until the
  // filter was turned off. `FILTERED_TOPUP_PAGES` bounds the other side.
  useEffect(() => {
    const el = listRef.current;
    if (!el || loadingHistory || historyExhausted.current) return;
    if (el.scrollHeight > el.clientHeight) return;
    if (markedOnly) {
      if (filteredTopUps.current >= FILTERED_TOPUP_PAGES) return;
      filteredTopUps.current += 1;
    }
    loadMoreHistory();
    // `keptIds`, not `lines`, is what puts a line on screen while filtering,
    // so it is what changes the height this decides on.
  }, [lines, keptIds, loadingHistory, fontPx, markedOnly]);

  // Re-pin to the bottom on a new line, and whenever the explain panel opens,
  // fills or closes — each resizes the pane. Respects a manual scroll-up.
  //
  // Keyed on the id of the newest line on screen, **not** on `lines`: judging a
  // word rebuilds that array without adding to it, so keyed on `lines` every tap
  // re-pinned and took the word out from under the finger. Prepended history is
  // excluded by the same key; `loadMoreHistory` restores the position itself.
  const newestVisibleId = visible.length ? visible[visible.length - 1].id : 0;
  useEffect(() => {
    pinToBottom(true);
  }, [newestVisibleId, explain, explaining, markedOnly]);

  /** Put the newest line back on the bottom edge, if the reader is following
   *  along (`stick`). `force` is for events that moved the bottom outright — a
   *  new line, the panel opening; everything else takes the height test. */
  function pinToBottom(force) {
    const el = listRef.current;
    if (!el || !stick.current) return;
    if (!force && el.scrollHeight === pinnedHeight.current) return;
    el.scrollTop = el.scrollHeight;
    pinnedHeight.current = el.scrollHeight;
  }

  // The other half of staying at the bottom: the feed *reflowing* under a reader
  // who never moved, which an id-keyed pin cannot see. Three things do it on an
  // ordinary load — the web font landing after first paint (`display=swap`), a
  // page of history arriving on top, and the pane being resized.
  //
  // The height test is what makes it safe to depend on `lines` here, which the
  // pin above must not: judging a word changes no height, so a tap is a no-op
  // and the word stays under the finger.
  useLayoutEffect(() => {
    pinToBottom(false);
  }, [lines, keptIds, fontPx, markedOnly]);

  useEffect(() => {
    // `display=swap` reflows every line when the font arrives, after the feed
    // has been painted and pinned. Resolves immediately on a warm cache.
    if (!document.fonts) return;
    let cancelled = false;
    document.fonts.ready.then(() => {
      if (!cancelled) pinToBottom(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Put the reader's place back after a page of history lands on top, by holding
  // one *line* still rather than summing heights.
  //
  // A height sum ran one frame after the prepend, a render too early under the
  // filter: `keptIds` had not admitted the new lines, the delta was zero, and
  // `scrollTop` stayed at 0. Pinned at the very top a browser fires no scroll
  // event, so the trigger for the next page could never run again.
  //
  // An anchor line has no deadline. It keys on `keptIds` as well as `lines` and
  // re-baselines, so it corrects on whichever render actually moves the line.
  // Declared above the paint, so marks measure against a settled scroll offset.
  useLayoutEffect(() => {
    const el = listRef.current;
    const anchor = historyAnchor.current;
    if (!el || !anchor) return;
    const anchorEl = lineEls.current.get(anchor.id);
    if (!anchorEl) return;
    const delta = anchorEl.offsetTop - anchor.offsetTop;
    if (!delta) return;
    el.scrollTop += delta;
    anchor.offsetTop = anchorEl.offsetTop;
  }, [lines, keptIds]);

  // What the painter reads, held in a ref rather than captured: the frame it
  // runs in is not the render that asked for it, and the observer below is
  // built once and must still see the current feed.
  const paintInput = useRef({ visible, highlightOn, commonRanks });
  paintInput.current = { visible, highlightOn, commonRanks };

  /** Ask for a repaint, at most one per frame.
   *
   *  The opening batch arrives as one SSE event per line, so 200 lines are 200
   *  renders, and painting measures every marked word on screen with its own
   *  `Range` + `getClientRects` — a forced layout each. Painting per render is
   *  quadratic in the batch, and now that almost every word is unjudged almost
   *  every token is measured rather than skipped as known.
   *
   *  Still flash-free: a rAF callback runs before the frame it was scheduled
   *  for is painted, so the marks land with the text, which is what the layout
   *  effect below was for. */
  const paintPending = useRef(false);
  const requestPaint = useCallback(() => {
    if (paintPending.current) return;
    paintPending.current = true;
    requestAnimationFrame(() => {
      paintPending.current = false;
      const { visible, highlightOn, commonRanks } = paintInput.current;
      paintMarks(
        visible,
        lineEls.current,
        highlightOn,
        listRef.current,
        marksRef.current,
        commonRanks,
      );
    });
  }, []);

  // Repaint whenever the text could have moved: a new line, a font change, the
  // toggle. A layout effect, not an effect — it measures just-committed nodes,
  // and a frame with text drawn but unmarked flashes on every line.
  //
  // **Depends on `keptIds`, not only `lines`.** `visible` derives from it and it
  // settles a render later: on a filtered backscroll the prepended page grew
  // `lines` and repainted against the old `keptIds`, then admitting those lines
  // pushed everything on screen down with nothing in the dependency list
  // changed. Marks sat a page-height off their words until an unrelated repaint
  // corrected them.
  useLayoutEffect(requestPaint, [
    lines,
    keptIds,
    highlightOn,
    fontPx,
    markedOnly,
    commonRanks,
  ]);

  // The other way the text reflows: the window or the split pane changing width.
  // Nothing re-renders then, so nothing above would fire — for the marks or for
  // the bottom edge, which a narrower pane pushes down as the lines wrap.
  //
  // **Must not depend on `lines`.** `observe` fires its callback immediately, so
  // rebuilding the observer per line would paint a second time for every line
  // on top of the effect above. `requestPaint` reads the current lines from a
  // ref, so one observer for the life of the view sees fresh ones anyway, and
  // `pinToBottom` only touches refs.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      requestPaint();
      pinToBottom(false);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [requestPaint]);

  useEffect(() => {
    // Successes clear themselves; failures stay until the next attempt.
    if (!toast || !toast.ok) return;
    // Both undoable toasts get the longer life: they are the only route back.
    const ms = toast.undo ? UNDO_TOAST_MS : TOAST_MS;
    const t = setTimeout(() => setToast(null), ms);
    return () => clearTimeout(t);
  }, [toast]);

  // Yomitan's {document-title} marker becomes the note's Document field, which
  // is how a card's source is tracked — so while reading, the page title has to
  // be the VN rather than the app's own. Restored on the way out so the
  // dashboard tab reads normally again.
  useEffect(() => {
    const workTitle = (state && state.current_work) || "";
    if (!workTitle) return;
    const previous = document.title;
    document.title = workTitle;
    return () => {
      document.title = previous;
    };
  }, [state && state.current_work]);

  function onScroll(e) {
    const el = e.currentTarget;
    stick.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < STICK_SLOP_PX;
    if (el.scrollTop < HISTORY_TRIGGER_PX) loadMoreHistory();
  }

  /** Pull one more page of history onto the top of the feed, for the reader
   *  who scrolled back looking for an earlier session. Fires from the scroll
   *  handler rather than an IntersectionObserver: the trigger is a plain
   *  distance from the top of one known scroller, and this needs no separate
   *  element to watch for it.
   *
   *  Prepending shifts every line below it, so the reader's place has to be put
   *  back or the browser leaves them looking at where the feed now *starts*.
   *  That is `historyAnchor`: the topmost line on screen is noted here, and the
   *  effect above keeps it where it was. */
  async function loadMoreHistory() {
    const el = listRef.current;
    const oldest = lines[0];
    if (!el || !oldest || loadingHistory || historyExhausted.current) return;
    // The topmost *rendered* line, which under the filter is not lines[0] —
    // a hidden line has no element to measure.
    const anchor = visible[0];
    const anchorEl = anchor && lineEls.current.get(anchor.id);
    setLoadingHistory(true);
    try {
      const r = await api(
        `/api/lines/before?before=${oldest.id}&limit=${HISTORY_PAGE}`,
      );
      if (!r.lines.length) {
        historyExhausted.current = true;
        return;
      }
      if (anchorEl) {
        historyAnchor.current = {
          id: anchor.id,
          offsetTop: anchorEl.offsetTop,
        };
      }
      setLines((prev) => [...r.lines, ...prev]);
    } catch {
      // A failed page just leaves the trigger zone armed for the next scroll.
    } finally {
      setLoadingHistory(false);
    }
  }

  /** Show only the lines with a word worth marking in them, and back.
   *
   *  The feed answers "what did I read"; filtered, it answers "where in it is
   *  there anything I have not judged known" — the question asked when scrolling
   *  back over a finished sitting, where most lines hold nothing new. It is a
   *  view over the same lines and nothing else: no line is cleared, nothing is
   *  fetched, and turning it off brings the feed back exactly as it was.
   *
   *  A line counts as marked by the tiers that are actually *painted*, so what
   *  the filter keeps is what the reader can see. `known` spans are sent for
   *  every judged word on the page, so counting them would keep nearly every
   *  line and the button would look broken. */
  function toggleMarkedOnly() {
    setMarkedOnly((on) => {
      localStorage.setItem(MARKED_ONLY_KEY, on ? "off" : "on");
      // Seeded here rather than left to the effect above, which would render one
      // empty frame first. Turning it *off* keeps nothing: the next turn-on is a
      // fresh question, and lines judged in the meantime should drop out then.
      setKeptIds(
        on ? new Set() : new Set(lines.filter(hasMark).map((l) => l.id)),
      );
      // A fresh budget for the fresh question: turning the filter on again over
      // a feed that has grown since should be allowed to fill the pane again.
      filteredTopUps.current = 0;
      return !on;
    });
  }

  function toggleHighlight() {
    setHighlightOn((on) => {
      localStorage.setItem(HIGHLIGHT_KEY, on ? "off" : "on");
      return !on;
    });
  }

  /** Judge the word under a tap — the one write the reading view makes.
   *
   *  Two states: anything marked becomes `known`, a word already known becomes
   *  `unknown`. **`new` and `seen` are unreachable by hand**, since they are
   *  what the ledger says before anyone has judged.
   *
   *  No toast — the mark is the report: it changes under the finger, and a
   *  failed write is the mark coming back. Optimistic, and applied to every
   *  occurrence on screen rather than the one tapped, since the same word three
   *  lines up carries the same assertion. */
  async function judgeAt(event) {
    if (!highlightOn) return;
    // A tap ending a selection is a lookup or an explain-focus, not a
    // judgement — `isCollapsed` tells them apart.
    const sel = window.getSelection?.();
    if (sel && !sel.isCollapsed) return;
    const hit = spanAtPoint(
      visible,
      lineEls.current,
      event.clientX,
      event.clientY,
    );
    if (!hit) return;
    const token = hit.token;
    const status = token.status === "known" ? "unknown" : "known";
    const term = { headword: token.headword, reading: token.reading };
    setLines((prev) => withStatus(prev, term, status));
    try {
      await api("/api/vocab/judge", {
        method: "POST",
        body: { judgements: [{ ...term, status }] },
      });
    } catch {
      // Put the mark back: the assertion did not land, and the mark returning
      // is the report, where the reader is already looking.
      setLines((prev) => withStatus(prev, term, token.status));
    }
  }

  function bumpFont(delta) {
    setFontPx((px) => {
      const next = Math.min(40, Math.max(13, px + delta));
      localStorage.setItem(FONT_KEY, String(next));
      return next;
    });
  }

  /** Send the last few lines to the model for a short read on the newest one.
   *  A selected word becomes the focus, captured first thing so the tap that
   *  opens this doesn't clear it, and reset afterwards. */
  async function explainLine() {
    const sel = (window.getSelection?.().toString() || "").trim();
    const context = lines.slice(-EXPLAIN_CONTEXT_LINES).map((l) => l.text);
    if (!context.length || explaining) return;
    setExplaining(true);
    setExplain({ focus: sel, text: "" });
    try {
      await streamExplain({
        context,
        focus: sel,
        // Shown as it arrives, so the panel fills rather than waiting on the
        // whole answer. `explaining` stays set until it is done, which is what
        // keeps the button disabled.
        onText: (text) => setExplain({ ok: true, focus: sel, text }),
      });
    } catch (err) {
      setExplain({ ok: false, focus: sel, text: err.message });
    } finally {
      setExplaining(false);
      window.getSelection?.().removeAllRanges();
    }
  }

  /** Drop the newest line from every derived figure. One tap per line, and the
   *  feed loses it as it goes. The id comes from what is on screen rather than
   *  the server picking "the last one", so a line hooked mid-tap is safe. */
  async function clearLast() {
    const line = lines[lines.length - 1];
    if (!line || clearing) return;
    setClearing(true);
    try {
      const r = await api("/api/lines/discard", {
        method: "POST",
        body: { ids: [line.id] },
      });
      if (!r.ids.length) return;
      setLines((prev) => prev.filter((l) => l.id !== line.id));
      // Consecutive taps accumulate into one undo batch.
      setToast((prev) => {
        const undo = [...((prev && prev.undo) || []), line];
        return { ok: true, undo, text: clearedText(undo.length) };
      });
    } catch (err) {
      setToast({ ok: false, text: err.message });
    } finally {
      setClearing(false);
    }
  }

  async function undoClear() {
    const batch = (toast && toast.undo) || [];
    if (!batch.length) return;
    try {
      await api("/api/lines/undiscard", {
        method: "POST",
        body: { ids: batch.map((l) => l.id) },
      });
      // The stream won't resend them (the client is past their ids), so they
      // go back in from the batch, in id order.
      setLines((prev) => {
        const byId = new Map(prev.map((l) => [l.id, l]));
        for (const l of batch) byId.set(l.id, l);
        return [...byId.values()].sort((a, b) => a.id - b.id);
      });
      setToast(null);
    } catch (err) {
      setToast({ ok: false, text: err.message });
    }
  }

  async function togglePause() {
    try {
      const r = await api("/api/capture/pause", { method: "POST", body: {} });
      setState((s) => ({ ...s, paused: r.paused }));
    } catch (err) {
      setToast({ ok: false, text: err.message });
    }
  }

  // Assembled here rather than inline: htm collapses the whitespace where
  // literal text meets an interpolation across a line break.
  const paused = state && state.paused;
  const workTitle = (state && state.current_work) || "";
  const work = workTitle || "no work set";
  // The page's own connection first: while it is down, the last status we hold
  // is stale and reporting it would be a guess.
  const captureState = live ? (capture ? capture.capture : "…") : "offline";
  const CAPTURE_BADGE = {
    live: ["live", "Lines are being hooked and recorded"],
    stalled: [
      "writer stuck",
      "Textractor is hooked, but the logger cannot write to the database — lines are held in memory and will land when it clears",
    ],
    unhooked: [
      "not hooked",
      "The logger is running but Textractor is not attached — nothing is being captured",
    ],
    down: [
      "logger down",
      "vn-ws-logger is not running — nothing is being captured. Restart it: systemctl --user restart vn-buffer",
    ],
    paused: ["paused", "Capture is paused — nothing is being recorded"],
    offline: ["reconnecting…", "Not connected — is kotodex-server reachable?"],
    "…": ["checking…", "Waiting for the logger's first heartbeat"],
  };
  const [liveLabel, liveTitle] = CAPTURE_BADGE[captureState];
  const capturing = captureState === "live";
  const explainLabel = explaining ? "explaining…" : "ℹ explain last line";
  const pauseLabel = paused ? "▶ resume capture" : "⏸ pause capture";
  const clearLabel = clearing ? "…" : "✕ clear last";
  const emptyLabel =
    markedOnly && lines.length
      ? "No marked words in the lines loaded — scroll up for more, or show every line."
      : capturing
        ? "Waiting for the next hooked line…"
        : liveTitle;
  const explainOff = state && state.explain_available === false;
  // Quality-only: mining still works, so this is a quiet hint, not a disable.
  const trimOff = state && state.trim_available === false;
  const trimTitle =
    "whisper-service is down — mined clips are VAD-trimmed but not narrowed to the single mined sentence";
  const pauseTitle = paused
    ? "Reconnect to Textractor and start recording lines again"
    : "Disconnect from Textractor — no lines are recorded at all while this is off";
  // Built whole rather than split around the focus word — htm collapses the
  // whitespace at a line break.
  const explainTitle =
    explain && explain.focus
      ? `“${explain.focus}” in the last line`
      : "the last line";
  // Deliberately loud: the feed goes silent while paused, so a forgotten pause
  // costs the lines themselves and nothing can recover them.
  const pausedBanner = "⏸ PAUSED — no lines are being recorded. Tap to resume.";
  const markedOnlyLabel = markedOnly ? "◍ marked" : "◌ marked";
  const markedOnlyTitle = markedOnly
    ? "Showing only lines with a marked word — tap to show every line"
    : "Show only the lines with a word you have not judged known";
  // Off while filtering: it drops the newest line, which the filter may be
  // hiding, and that would clear something not on screen.
  const clearTitle = markedOnly
    ? "Show every line first — this drops the newest one, which the filter may be hiding"
    : "Drop the newest line from the stats — lines hooked while finding the route, or a stretch re-read after skipping back";
  const highlightLabel = highlightOn ? "◨ words" : "◫ words";
  const highlightTitle = highlightOn
    ? "Tinting words you have not judged known — tap to read the line plain"
    : "Tint words you have not judged known";

  return html`
    <div class="reader ${paused ? "is-paused" : ""}">
      <div class="reader-bar">
        <a class="reader-back" href="#" title="Back to the dashboard">←</a>
        <span class="reader-work">${work}</span>
        <span class="reader-live ${capturing ? "on" : "off"}" title=${liveTitle}
          >${liveLabel}</span
        >
        ${
          trimOff &&
          html`<span class="reader-trimoff" title=${trimTitle}>✂ off</span>`
        }
        <button
          class="ghost ${markedOnly ? "on" : ""}"
          onClick=${toggleMarkedOnly}
          title=${markedOnlyTitle}
        >
          ${markedOnlyLabel}
        </button>
        <button
          class="ghost ${highlightOn ? "on" : ""}"
          onClick=${toggleHighlight}
          title=${highlightTitle}
        >
          ${highlightLabel}
        </button>
        <button class="ghost" onClick=${() => bumpFont(-2)}>A−</button>
        <button class="ghost" onClick=${() => bumpFont(2)}>A+</button>
      </div>
      ${
        paused &&
        html`<button class="reader-paused" onClick=${togglePause}>
          ${pausedBanner}
        </button>`
      }
      <div class="reader-feed">
        <div
          class="reader-lines"
          ref=${listRef}
          onScroll=${onScroll}
          onClick=${judgeAt}
          style=${`font-size: ${fontPx}px`}
        >
          <div class="reader-marks" ref=${marksRef} aria-hidden="true"></div>
          ${visible.length === 0 && html`<p class="reader-empty">${emptyLabel}</p>`}
          ${renderFeed(
            visible,
            lineEls.current,
            (state && state.session_gap_secs) || DEFAULT_SESSION_GAP_SECS,
          )}
        </div>
        ${
          toast &&
          html`<div class="reader-toast ${toast.ok ? "ok" : "err"}">
            <span>${toast.text}</span>
            ${
              toast.undo &&
              html`<button class="reader-undo" onClick=${undoClear}>
                undo
              </button>`
            }
          </div>`
        }
      </div>
      ${
        (explaining || explain) &&
        html`<div
          class="reader-explain ${explain && explain.ok === false ? "err" : ""}"
        >
          <div class="reader-explain-head">
            <span class="reader-explain-title">Explain: ${explainTitle}</span>
            <button
              class="reader-explain-close"
              onClick=${() => setExplain(null)}
              title="Dismiss"
            >
              ✕
            </button>
          </div>
          <div class="reader-explain-body">
            ${explain && explain.text ? renderMarkdown(explain.text) : "explaining…"}
          </div>
        </div>`
      }
      <div class="reader-actions">
        <button
          class="reader-pause ${paused ? "paused" : ""}"
          onClick=${togglePause}
          title=${pauseTitle}
        >
          ${pauseLabel}
        </button>
        <button
          class="reader-clear"
          disabled=${clearing || lines.length === 0 || markedOnly}
          onClick=${clearLast}
          title=${clearTitle}
        >
          ${clearLabel}
        </button>
        <button
          class="reader-explain-btn"
          disabled=${explaining || explainOff || lines.length === 0}
          onClick=${explainLine}
          title="Explain the last line (select a word first to focus on it)"
        >
          ${explainLabel}
        </button>
      </div>
    </div>
  `;
}

/** The model's Markdown as vnodes. The parse is `web-shared/markdown.js`, which
 *  the overlay reads too; only this mapping to vnodes is Preact's. Never a
 *  parser plus innerHTML, which would open an XSS seam on model output. */
function renderMarkdown(src) {
  return parseMarkdown(src).map((block, i) =>
    block.type === "ul"
      ? html`<ul key=${i}>
          ${block.items.map((item, j) => html`<li key=${j}>${inlineMd(item)}</li>`)}
        </ul>`
      : html`<p key=${i}>${inlineMd(block.spans)}</p>`,
  );
}

function inlineMd(spans) {
  return spans.map((s, i) => {
    if (s.style === "bold") return html`<strong key=${i}>${s.text}</strong>`;
    if (s.style === "italic") return html`<em key=${i}>${s.text}</em>`;
    return s.text;
  });
}

/** "cleared 3 lines", built whole since htm collapses the whitespace at a line
 *  break between literal text and an interpolation. */
function clearedText(n) {
  return `cleared ${n} ${n === 1 ? "line" : "lines"}`;
}

/** The feed as a flat run of `<p>`s with a header before each sitting, split on
 *  the same `sessionGapSecs` rule `stats::derive_sessions` uses, so a header
 *  here agrees with what the dashboard calls the same sitting.
 *
 *  Grouped and flattened right back: `paintMarks`, `spanAtPoint` and
 *  `withStatus` all index `lines` directly, so only this render pass knows
 *  where the sessions divide. */
function renderFeed(lines, els, sessionGapSecs) {
  const groups = groupSessions(lines, sessionGapSecs);
  const out = [];
  groups.forEach((g, i) => {
    // Recomputed every render, so the last group's end time keeps up — there
    // is no "closed" flag, only the next line's absence so far.
    const ongoing = i === groups.length - 1;
    out.push(
      html`<div class="reader-session-header" key=${`h${g.lines[0].id}`}>
        ${sessionHeaderLabel(g.start_ts, g.end_ts, ongoing)}
      </div>`,
    );
    for (const line of g.lines) out.push(renderLine(line, els));
  });
  return out;
}

/** Split a time-ordered run of lines into sittings the same way the server's
 *  `stats::derive_sessions` does: a gap over `sessionGapSecs` between two
 *  consecutive lines starts a new one. */
function groupSessions(lines, sessionGapSecs) {
  const groups = [];
  for (const line of lines) {
    const g = groups[groups.length - 1];
    if (!g || line.ts - g.end_ts > sessionGapSecs) {
      groups.push({ start_ts: line.ts, end_ts: line.ts, lines: [line] });
    } else {
      g.end_ts = line.ts;
      g.lines.push(line);
    }
  }
  return groups;
}

/** "started 14:32" for the sitting still being read, "14:32–15:07" for one that
 *  closed. Built whole, since htm collapses the whitespace at a line break. */
function sessionHeaderLabel(startTs, endTs, ongoing) {
  return ongoing
    ? `started ${fmtTime(startTs)}`
    : `${fmtTime(startTs)}–${fmtTime(endTs)}`;
}

/** "14:32". Pinned to en-GB like the sittings table: the default locale adds an
 *  AM/PM, and these headers sit in a pane kept narrow beside the VN. */
function fmtTime(ts) {
  return new Date(ts * 1000).toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** One line of the feed.
 *
 *  Its own function for one reason: the `<p>` must have exactly one child and
 *  that child must be the line's text node. `paintMarks` builds Ranges by
 *  offset into it, and a stray whitespace node in front — which is what htm
 *  yields when a template puts an interpolation on its own line — would shift
 *  every tint on the line left by one word. So the template stays on one line;
 *  don't wrap it. */
function renderLine(line, els) {
  return html`<p class="reader-line" key=${line.id} ref=${(el) => keepLineEl(els, line.id, el)}>${line.text}</p>`;
}

/** Track (or forget) the element holding a line. Preact calls the ref with
 *  `null` as a row leaves, so the map holds exactly what is on screen. */
function keepLineEl(els, id, el) {
  if (el) els.set(id, el);
  else els.delete(id);
}

/** Whether a line holds a word the feed would tint — the filter's whole rule.
 *
 *  Tested against PAINTED rather than "has any token": `known` spans are sent
 *  for every judged word, so any-token would keep nearly every line. */
function hasMark(line) {
  return (line.tokens || []).some((t) => PAINTED.includes(t.status));
}

/** Whether a word is common enough that not knowing it is a real gap. Asked
 *  only of words that get a mark at all, so being unknown is already settled.
 *
 *  A word neither list ranks is never common — the underline says "you should
 *  have this one", and an unranked word is the case where that cannot be
 *  claimed. The two lists are asked separately rather than reconciled: they
 *  disagree about what fiction and what prose use, and passing either is
 *  enough. */
function isCommonGap(token, commonRanks) {
  return (
    underRank(token.freq_rank, commonRanks.freq) ||
    underRank(token.bccwj_rank, commonRanks.bccwj)
  );
}

/** A rank at or under a threshold, with 0 meaning the threshold is off. */
function underRank(rank, max) {
  return max > 0 && rank != null && rank <= max;
}

/** Underlines nothing, for before the settings arrive. */
const NO_COMMON_RANKS = { freq: 0, bccwj: 0 };

/** Draw the marks: one rounded, padded rectangle behind each flagged word, in a
 *  layer underneath the text.
 *
 *  **Behind rather than on** — the text nodes are never touched, so Yomitan
 *  scans the same one-text-node-per-line it always did. Being ordinary elements
 *  rather than `::highlight()` pseudos, the marks take a border radius and
 *  padding, which that API cannot express. The rectangles come from the Ranges'
 *  client rects, so they track the text to the pixel.
 *
 *  Measured against the scroll *content*, not the viewport, so the layer scrolls
 *  with the lines and nothing runs on scroll — only on a reflow.
 *
 *  Every rect is measured before a node is inserted: interleaving reads and
 *  writes would force a layout per mark, on the path that runs while a line is
 *  being read. */
function paintMarks(lines, els, enabled, listEl, layerEl, commonRanks) {
  if (!listEl || !layerEl) return;
  const boxes = [];
  if (enabled) {
    const base = listEl.getBoundingClientRect();
    // Viewport → content coordinates: the layer is positioned against the
    // scroll container's padding box, which the scroll offsets undo.
    const dx = listEl.scrollLeft - base.left;
    const dy = listEl.scrollTop - base.top;
    for (const line of lines) {
      const el = els.get(line.id);
      const node = el && el.firstChild;
      // Only a lone text node is safe to offset into — see `renderLine`.
      if (!node || node.nodeType !== Node.TEXT_NODE) continue;
      for (const t of line.tokens || []) {
        // Offsets are UTF-16 code units, which is what a Range indexes in. A
        // span past the end means text and tokens came from different reads.
        if (!PAINTED.includes(t.status) || t.start + t.len > node.length)
          continue;
        const range = document.createRange();
        range.setStart(node, t.start);
        range.setEnd(node, t.start + t.len);
        // One rect per line box, so a wrapped word is marked on each
        // fragment rather than by one box spanning the gap.
        for (const r of range.getClientRects()) {
          if (!r.width) continue;
          boxes.push({
            tier: t.status,
            common: isCommonGap(t, commonRanks),
            left: r.left + dx + MARK_INSET_PX,
            top: r.top + dy,
            width: Math.max(1, r.width - MARK_INSET_PX * 2),
            height: r.height,
          });
        }
      }
    }
  }
  // One write after every read. `replaceChildren` also covers the disabled and
  // unmount cases, where `boxes` is empty.
  layerEl.replaceChildren(
    ...boxes.map((b) => {
      const div = document.createElement("div");
      div.className = `reader-mark ${b.tier}${b.common ? " common" : ""}`;
      div.style.cssText = `left:${b.left}px;top:${b.top}px;width:${b.width}px;height:${b.height}px;--underline-px:${UNDERLINE_PX}px`;
      return div;
    }),
  );
}

/** The span under a point, or null if the point isn't on a marked word.
 *
 *  Hit-tested against the text with `caretPositionFromPoint`. **Nothing in the
 *  feed is made clickable**: an interactive layer would sit between the reader
 *  and the text Yomitan scans, and a mark that swallowed a long-press would cost
 *  a lookup to gain a judgement.
 *
 *  A tap on anything unmarked finds no span and does nothing, which is the
 *  intended silence. */
function spanAtPoint(lines, els, x, y) {
  const caret = caretAt(x, y);
  if (!caret) return null;
  for (const line of lines) {
    const el = els.get(line.id);
    const node = el && el.firstChild;
    if (node !== caret.node) continue;
    for (const token of line.tokens || []) {
      const inside =
        caret.offset >= token.start && caret.offset < token.start + token.len;
      if (inside) return { line, token };
    }
    return null;
  }
  return null;
}

/** The text node and offset under a point, across the two spellings browsers
 *  give this: the standard `caretPositionFromPoint` and WebKit's older
 *  `caretRangeFromPoint`. */
function caretAt(x, y) {
  if (document.caretPositionFromPoint) {
    const pos = document.caretPositionFromPoint(x, y);
    if (!pos) return null;
    return { node: pos.offsetNode, offset: pos.offset };
  }
  if (document.caretRangeFromPoint) {
    const range = document.caretRangeFromPoint(x, y);
    if (!range) return null;
    return { node: range.startContainer, offset: range.startOffset };
  }
  return null;
}

/** Every occurrence of one term on screen, restatused — one word is one
 *  assertion, and leaving a copy marked would read as a failed write. */
function withStatus(lines, term, status) {
  return lines.map((line) => {
    if (!line.tokens || !line.tokens.some((t) => sameTerm(t, term)))
      return line;
    return {
      ...line,
      tokens: line.tokens.map((t) =>
        sameTerm(t, term) ? { ...t, status } : t,
      ),
    };
  });
}

function sameTerm(token, term) {
  return token.headword === term.headword && token.reading === term.reading;
}
