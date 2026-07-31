/** The reading view: the live Textractor line feed, sized to sit beside the
 *  running VN.
 *
 *  There is no mine button. Adding a card in Yomitan goes through this server's
 *  AnkiConnect proxy, and the proxy fires vn-capture.sh itself once Anki has
 *  accepted the note — so the audio and screenshot attach to every mine
 *  automatically, including the ones made from the desktop. A button would only
 *  be a second, manual way to do what already happened.
 *
 *  Lines are plain text nodes on purpose — Yomitan scans the DOM, so anything
 *  clever here (virtualized rows, per-token spans) would break lookups. The
 *  word marks are painted *around* that rule rather than through it: they are a
 *  layer of rectangles behind the text, never markup inside it. See
 *  `paintMarks` at the foot of this file. */
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { html } from "htm/preact";
import { api } from "./api.js";

const FONT_KEY = "reader-font-px";
const FONT_DEFAULT = 20;
const HIGHLIGHT_KEY = "reader-highlight";
const MARKED_ONLY_KEY = "reader-marked-only";
/** The statuses that get a colour. `known` is sent too — a span is also the
 *  region a tap judges, so a word just marked known must stay tappable — but it
 *  is deliberately absent here: on a page where most words are known, the
 *  absence of a mark is what makes the marks readable. Anything unrecognised is
 *  ignored rather than drawn in a default colour; that is a version skew, not a
 *  word. */
const PAINTED = ["seen", "new", "unknown"];
/** How far a mark reaches past its word on each side. Enough to look like a
 *  deliberate band rather than a tight box, small enough that two marked words
 *  next to each other don't merge into one. */
const MARK_PAD_PX = 2;
/** Distance from the bottom that still counts as "following along", so
 *  scrolling up to re-read isn't yanked back by the next line. */
const STICK_SLOP_PX = 80;
const TOAST_MS = 6000;
/** Longer than an ordinary toast: this one is the only route back to a cleared
 *  line, and clearing a handful of them is several taps. */
const UNDO_TOAST_MS = 15000;
/** How often the work title / pause state are re-read. Slow enough to be free,
 *  fast enough that switching works on the dashboard lands before the next
 *  card is mined. */
const STATE_POLL_MS = 20_000;
/** Lines sent to the model with the last one to explain. Enough to place a
 *  pronoun or an unstated subject without turning a quick read into a scene
 *  dump; the server caps it again in case the feed grows. */
const EXPLAIN_CONTEXT_LINES = 8;
/** Used to group the feed into sessions before `/api/reader/state` answers,
 *  so the very first paint doesn't show one header for everything. Replaced
 *  the moment the real setting arrives. */
const DEFAULT_SESSION_GAP_SECS = 600;
/** One backscroll page, matching the server's default so a full page really
 *  is more history rather than a partial one that reads as "nothing left". */
const HISTORY_PAGE = 200;
/** How close to the top of the scroller counts as "reaching for more
 *  history" — close enough that the fetch lands before the reader gets there. */
const HISTORY_TRIGGER_PX = 300;

export function Reader() {
  const [lines, setLines] = useState([]);
  const [live, setLive] = useState(false);
  const [state, setState] = useState(null);
  const [clearing, setClearing] = useState(false);
  const [explaining, setExplaining] = useState(false);
  const [explain, setExplain] = useState(null);
  const [toast, setToast] = useState(null);
  const [loadingHistory, setLoadingHistory] = useState(false);
  /** Set once a backscroll page comes back empty — there is nothing older, so
   *  the trigger stops firing. A ref rather than state: it only ever gates the
   *  next fetch, and nothing on screen changes when it flips. */
  const historyExhausted = useRef(false);
  const [fontPx, setFontPx] = useState(
    () => Number(localStorage.getItem(FONT_KEY)) || FONT_DEFAULT,
  );
  // Highlighting is on unless it was turned off, and the browser can paint it.
  // A browser without the Highlight API isn't offered the toggle at all rather
  // than being given one that does nothing.
  const [highlightOn, setHighlightOn] = useState(
    () => localStorage.getItem(HIGHLIGHT_KEY) !== "off",
  );
  // Off by default and remembered: it is a way of looking back over what has
  // been read, not a way of reading. Persisted like the font and the tinting
  // so a reload beside the VN comes back the way it was left.
  const [markedOnly, setMarkedOnly] = useState(
    () => localStorage.getItem(MARKED_ONLY_KEY) === "on",
  );
  /** The ids the filter is letting through: every line that has *had* a marked
   *  word in it since the filter was turned on. It only ever grows while the
   *  filter is on, and is rebuilt from scratch each time it is turned on again.
   *
   *  Membership rather than a live predicate, because judging the last marked
   *  word in a line would otherwise delete that line from under the finger that
   *  judged it — the feed shifting by a line at the moment of a tap, which is
   *  the thing the reader is looking at. A judged line staying put, plain, is
   *  also the report: the mark is gone and the line is still there. Toggling the
   *  filter off and on is what clears them out. */
  const [keptIds, setKeptIds] = useState(() => new Set());
  const listRef = useRef(null);
  const stick = useRef(true);
  /** line id → the <p> holding it, for the Ranges the highlights are built
   *  from. A Range needs the text node itself, so the elements have to be
   *  reachable outside the render that made them. */
  const lineEls = useRef(new Map());
  /** The layer the marks are drawn into. Outside Preact's reconciliation on
   *  purpose: it is decorative, it is rewritten wholesale on every repaint, and
   *  routing a few hundred absolutely-positioned rectangles through the vdom
   *  would re-render the feed to move a box. */
  const marksRef = useRef(null);

  /** The lines actually on screen. `lines` stays the whole feed whatever the
   *  filter is doing — judging a word rewrites every occurrence in it, including
   *  the ones a filter is hiding, and a mark that came back when the filter was
   *  cleared would read as a write that failed. So the filter is applied at the
   *  last possible moment, and everything that measures or hit-tests the text
   *  (`paintMarks`, `spanAtPoint`) is given *this* list rather than `lines`:
   *  those index into elements, and a hidden line has none. */
  const visible = markedOnly ? lines.filter((l) => keptIds.has(l.id)) : lines;

  useEffect(() => {
    // EventSource reconnects on its own and replays from Last-Event-ID, so a
    // backgrounded tab or a slept screen resumes without losing lines.
    const es = new EventSource("/api/lines/stream");
    es.onopen = () => setLive(true);
    es.onerror = () => setLive(false);
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
    // Polled rather than fetched once: current_work drives the document title
    // that ends up on the card (below), and pausing from the desktop hotkey
    // should show here too. Both are cheap reads.
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

  // A feed shorter than its pane never fires a scroll event, so the trigger
  // above can't be reached and the sessions above it would be stranded —
  // which is exactly the case of a sitting that has just started. Pull pages
  // until there is something to scroll, or there is no more history.
  useEffect(() => {
    const el = listRef.current;
    if (!el || loadingHistory || historyExhausted.current) return;
    // Not while filtering. A filtered feed is short by construction, so this
    // would page back through the entire history a hundred lines at a time
    // trying to fill a pane that most lines are excluded from. Scrolling still
    // pulls more; only the automatic top-up is off.
    if (markedOnly) return;
    if (el.scrollHeight > el.clientHeight) return;
    loadMoreHistory();
  }, [lines, loadingHistory, fontPx, markedOnly]);

  // Re-pin to the bottom on a new line, and also whenever the explain panel
  // opens, fills in, or closes — each resizes the lines pane, and without this
  // the newest line would slip out of view above the panel instead of sitting
  // right on top of it. Respects a manual scroll-up (stick=false) either way.
  //
  // Keyed on the id of the newest line on screen, *not* on `lines`. Judging a
  // word rebuilds that array without adding anything to it, so depending on it
  // made every tap re-pin: scroll back a couple of lines — still inside
  // STICK_SLOP_PX, so still "following along" — tap a word, and the feed jumped
  // to the bottom, moving the word out from under the finger that had just
  // judged it. Prepended history is excluded for the same reason: the id at the
  // bottom has not changed, and `loadMoreHistory` restores the position itself.
  const newestVisibleId = visible.length ? visible[visible.length - 1].id : 0;
  useEffect(() => {
    if (stick.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [newestVisibleId, explain, explaining, markedOnly]);

  // Repaint whenever the text could have moved: a new line, a font change, the
  // toggle. Layout effect rather than effect — this measures nodes that were
  // just committed, and a frame with the text drawn but not yet marked is a
  // flash on every single line.
  useLayoutEffect(() => {
    paintMarks(
      visible,
      lineEls.current,
      highlightOn,
      listRef.current,
      marksRef.current,
    );
  }, [lines, highlightOn, fontPx, markedOnly]);

  // The other way the text reflows: the window, or the split pane beside the
  // VN, changing width. Nothing re-renders then, so nothing above would fire.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() =>
      paintMarks(visible, lineEls.current, highlightOn, el, marksRef.current),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, [lines, highlightOn, fontPx, markedOnly]);

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
  // be the VN rather than "read-stats". Restored on the way out so the
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
   *  Prepending shifts every line below it, so the scroll position is
   *  restored from the height *added* rather than pinned to an id — the
   *  browser would otherwise yank the view down to where the feed now starts. */
  async function loadMoreHistory() {
    const el = listRef.current;
    const oldest = lines[0];
    if (!el || !oldest || loadingHistory || historyExhausted.current) return;
    setLoadingHistory(true);
    try {
      const r = await api(
        `/api/lines/before?before=${oldest.id}&limit=${HISTORY_PAGE}`,
      );
      if (!r.lines.length) {
        historyExhausted.current = true;
        return;
      }
      const prevHeight = el.scrollHeight;
      const prevTop = el.scrollTop;
      setLines((prev) => [...r.lines, ...prev]);
      // Runs after the DOM has the new rows (state update flushed before the
      // next paint), so the height delta is measured against the final layout.
      requestAnimationFrame(() => {
        el.scrollTop = prevTop + (el.scrollHeight - prevHeight);
      });
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
      return !on;
    });
  }

  function toggleHighlight() {
    setHighlightOn((on) => {
      localStorage.setItem(HIGHLIGHT_KEY, on ? "off" : "on");
      return !on;
    });
  }

  /** Judge the word under a tap.
   *
   *  The one write the reading view makes to the ledger, and it exists because
   *  the marks put the question right where the reader already is: the word is
   *  in front of them, in context, at the moment they know the answer.
   *
   *  Two states, and they are the two a reader can answer without leaving the
   *  line: anything marked becomes known, and a word already known becomes
   *  unknown. `new` and `seen` are not among them — they are what the ledger
   *  says *before* anyone has judged, so writing one by hand would be asserting
   *  that nothing has been asserted. Tapping past a mistake is one more tap.
   *
   *  No toast: the mark itself is the report. It changes colour or goes away
   *  under the finger that asked, which is both faster to read than a line of
   *  text and impossible to miss — and a failed write is the mark coming back.
   *
   *  Optimistic, and applied to every occurrence of the term on screen rather
   *  than the one tapped — the same word three lines up is the same assertion,
   *  and leaving it marked would read as a failed write. */
  async function judgeAt(event) {
    if (!highlightOn) return;
    // A tap that ends a selection is a lookup or an explain-focus, not a
    // judgement. `isCollapsed` is what tells those apart.
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
      // Put the mark back. The assertion did not land, and a view that shows it
      // as though it did is worse than any message about it — the mark
      // returning is the report, in the place the reader is already looking.
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
   *  A word selected in the feed becomes the focus — captured first thing, so
   *  the tap that opens this doesn't matter, and cleared afterwards so the next
   *  explain doesn't reuse a stale one. */
  async function explainLine() {
    const sel = (window.getSelection?.().toString() || "").trim();
    const context = lines.slice(-EXPLAIN_CONTEXT_LINES).map((l) => l.text);
    if (!context.length || explaining) return;
    setExplaining(true);
    setExplain({ focus: sel, text: "" });
    try {
      const r = await api("/api/reader/explain", {
        method: "POST",
        body: { context, focus: sel },
      });
      setExplain({ ok: true, focus: sel, text: r.text });
    } catch (err) {
      setExplain({ ok: false, focus: sel, text: err.message });
    } finally {
      setExplaining(false);
      window.getSelection?.().removeAllRanges();
    }
  }

  /** Drop the newest line from every derived figure. One tap per line: the
   *  feed loses it as it goes, so tapping until the junk is gone needs no
   *  count in the UI. The id comes from what is on screen rather than the
   *  server picking "the last one", so a line hooked mid-tap isn't swept up. */
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
      // Consecutive taps accumulate into one undo batch, so clearing five
      // stray lines is still a single way back.
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
      // The stream won't resend them — the client is already past their ids —
      // so they go back in from the batch, in id order.
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

  // Sub-strings are assembled here rather than inline: htm collapses the
  // whitespace where literal text meets an interpolation across a line break.
  const paused = state && state.paused;
  const workTitle = (state && state.current_work) || "";
  const work = workTitle || "no work set";
  const liveLabel = live ? "live" : "reconnecting…";
  const explainLabel = explaining ? "explaining…" : "ℹ explain last line";
  const pauseLabel = paused ? "▶ resume capture" : "⏸ pause capture";
  const clearLabel = clearing ? "…" : "✕ clear last";
  const emptyLabel =
    markedOnly && lines.length
      ? "No marked words in the lines loaded — scroll up for more, or show every line."
      : live
        ? "Waiting for the next hooked line…"
        : "Not connected — is read-stats reachable?";
  const explainOff = state && state.explain_available === false;
  // Quality-only: mining still works, so this is a quiet hint, not a disable.
  const trimOff = state && state.trim_available === false;
  const trimTitle =
    "whisper-service is down — mined clips are VAD-trimmed but not narrowed to the single mined sentence";
  const pauseTitle = paused
    ? "Reconnect to Textractor and start recording lines again"
    : "Disconnect from Textractor — no lines are recorded at all while this is off";
  // Built whole rather than split around the focus word — htm collapses the
  // whitespace where literal text meets an interpolation across a line break.
  const explainTitle =
    explain && explain.focus
      ? `“${explain.focus}” in the last line`
      : "the last line";
  // Deliberately loud, and more so than when a pause merely voided the span:
  // the feed goes silent while paused, so a forgotten pause costs the lines
  // themselves rather than just their credit. Nothing can recover them.
  const pausedBanner = "⏸ PAUSED — no lines are being recorded. Tap to resume.";
  const markedOnlyLabel = markedOnly ? "◍ marked" : "◌ marked";
  const markedOnlyTitle = markedOnly
    ? "Showing only lines with a marked word — tap to show every line"
    : "Show only the lines with a word you have not judged known";
  // Off while filtering: it drops *the newest line*, which the filter may well
  // be hiding, and a button that discards something not on screen is a button
  // that clears the wrong line.
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
        <span class="reader-live ${live ? "on" : "off"}">${liveLabel}</span>
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
            ${explaining ? "explaining…" : renderMarkdown(explain.text)}
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

/** Render the small slice of Markdown the model emits — paragraphs, `-`/`*`
 *  bullet lists, and `**bold**` / `*italic*` inline — as vnodes. Deliberately
 *  not a CDN parser + innerHTML: that would be more than this needs and open an
 *  XSS seam on model output; this covers exactly what comes back. */
function renderMarkdown(src) {
  const blocks = (src || "").trim().split(/\n{2,}/);
  return blocks.map((block, i) => {
    const rows = block.split("\n");
    const isList = rows.length > 0 && rows.every((l) => /^\s*[-*]\s+/.test(l));
    if (isList) {
      return html`<ul key=${i}>
        ${rows.map(
          (l, j) =>
            html`<li key=${j}>${inlineMd(l.replace(/^\s*[-*]\s+/, ""))}</li>`,
        )}
      </ul>`;
    }
    // Soft-wrapped lines in one block are one paragraph.
    return html`<p key=${i}>${inlineMd(rows.join(" "))}</p>`;
  });
}

/** `**bold**` and `*italic*` spans within a line, everything else literal. */
function inlineMd(text) {
  const parts = [];
  const re = /\*\*([^*]+)\*\*|\*([^*]+)\*/g;
  let last = 0;
  let key = 0;
  let m;
  while ((m = re.exec(text))) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    if (m[1] != null) parts.push(html`<strong key=${key++}>${m[1]}</strong>`);
    else parts.push(html`<em key=${key++}>${m[2]}</em>`);
    last = re.lastIndex;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

/** "cleared 3 lines" — built whole rather than split around the count, since htm
 *  collapses the whitespace where literal text meets an interpolation. */
function clearedText(n) {
  return `cleared ${n} ${n === 1 ? "line" : "lines"}`;
}

/** The feed as a flat run of `<p>`s with a header before each sitting — a
 *  gap over `sessionGapSecs` between two consecutive lines starts a new one,
 *  the same rule `stats::derive_sessions` splits sessions on server-side, so
 *  a header here agrees with what the dashboard would call the same sitting.
 *
 *  Grouped and flattened right back rather than kept nested: `paintMarks`,
 *  `spanAtPoint` and `withStatus` all index `lines` directly, and the
 *  underlying `lines` state stays that flat array — only this render pass
 *  needs to know where the sessions divide. */
function renderFeed(lines, els, sessionGapSecs) {
  const groups = groupSessions(lines, sessionGapSecs);
  const out = [];
  groups.forEach((g, i) => {
    // Recomputed every render, so the last group's end time keeps up as its
    // session is still being read — there is no "closed" flag from the
    // server, only the next line's absence so far.
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

/** "started 14:32" for the sitting still being read — there is no end time
 *  yet, only the last line so far — and "14:32–15:07" for one that closed
 *  because another began after it. Built whole rather than split around the
 *  times, since htm collapses the whitespace where literal text meets an
 *  interpolation across a line break. */
function sessionHeaderLabel(startTs, endTs, ongoing) {
  return ongoing
    ? `started ${fmtTime(startTs)}`
    : `${fmtTime(startTs)}–${fmtTime(endTs)}`;
}

/** "14:32". Pinned to en-GB rather than the browser's locale, the same way the
 *  sittings table does it: the default locale puts an AM/PM on it, and these
 *  headers sit in a pane kept narrow beside the VN. */
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
 *  every tint on the line left by one word. Hence the `prettier-ignore`: this
 *  one line must not be reflowed. */
// prettier-ignore
function renderLine(line, els) {
  return html`<p class="reader-line" key=${line.id} ref=${(el) => keepLineEl(els, line.id, el)}>${line.text}</p>`;
}

/** Track (or forget) the element holding a line. Preact calls the ref with
 *  `null` as a row leaves, so the map holds exactly the lines currently on
 *  screen — which is now the whole session and whatever history was scrolled
 *  back into, rather than a fixed window of it. */
function keepLineEl(els, id, el) {
  if (el) els.set(id, el);
  else els.delete(id);
}

/** Whether a line holds a word the feed would tint — the filter's whole rule.
 *
 *  Tested against PAINTED rather than "has any token": every judged word on the
 *  line arrives as a `known` span too, so any-token would keep nearly every line
 *  and the filter would look broken. */
function hasMark(line) {
  return (line.tokens || []).some((t) => PAINTED.includes(t.status));
}

/** Draw the marks: one rounded, padded rectangle behind each word the server
 *  flagged, in a layer of its own underneath the text.
 *
 *  Behind rather than on, and this is the whole design. The text nodes are
 *  never touched — Yomitan scans the same one-text-node-per-line it always did,
 *  which is the invariant this file's header states. But because the marks are
 *  ordinary elements rather than `::highlight()` pseudos, they take a border
 *  radius and horizontal padding, which that API cannot express at all. The
 *  rectangles come from the Ranges' own client rects, so they track the text to
 *  the pixel without the text knowing they exist.
 *
 *  Positions are measured relative to the scroll *content*, not the viewport,
 *  so the layer scrolls with the lines and nothing has to run on scroll. It
 *  does have to run whenever the text could have reflowed: a new line, the font
 *  size, a resize.
 *
 *  Every rect is measured before a single node is inserted. Interleaving reads
 *  and writes here would force a layout per mark, on the one path that runs
 *  while a line is being read. */
function paintMarks(lines, els, enabled, listEl, layerEl) {
  if (!listEl || !layerEl) return;
  const boxes = [];
  if (enabled) {
    const base = listEl.getBoundingClientRect();
    // Viewport → content coordinates. The layer is positioned against the
    // scroll container's padding box, which is what the scroll offsets undo.
    const dx = listEl.scrollLeft - base.left;
    const dy = listEl.scrollTop - base.top;
    for (const line of lines) {
      const el = els.get(line.id);
      const node = el && el.firstChild;
      // Only a lone text node is safe to offset into — see `renderLine`.
      if (!node || node.nodeType !== Node.TEXT_NODE) continue;
      for (const t of line.tokens || []) {
        // The offsets are UTF-16 code units from the server, which is what a
        // Range indexes in. A span past the end means the text and the tokens
        // came from different reads of the line; skip it rather than throw.
        if (!PAINTED.includes(t.status) || t.start + t.len > node.length)
          continue;
        const range = document.createRange();
        range.setStart(node, t.start);
        range.setEnd(node, t.start + t.len);
        // One rect per line box: a word broken across a wrap gets a mark on
        // each fragment rather than one box spanning the gap between them.
        for (const r of range.getClientRects()) {
          if (!r.width) continue;
          boxes.push({
            tier: t.status,
            left: r.left + dx - MARK_PAD_PX,
            top: r.top + dy,
            width: r.width + MARK_PAD_PX * 2,
            height: r.height,
          });
        }
      }
    }
  }
  // One write, after every read. `replaceChildren` also covers the disabled
  // and unmount cases, where `boxes` is simply empty.
  layerEl.replaceChildren(
    ...boxes.map((b) => {
      const div = document.createElement("div");
      div.className = `reader-mark ${b.tier}`;
      div.style.cssText = `left:${b.left}px;top:${b.top}px;width:${b.width}px;height:${b.height}px`;
      return div;
    }),
  );
}

/** The span under a point, or null if the point isn't on a marked word.
 *
 *  Hit-tested against the text itself — `caretPositionFromPoint` gives the text
 *  node and offset under the finger, and the offsets already on screen say
 *  which word that is. Nothing in the feed is made clickable to achieve it:
 *  an interactive layer over the lines would sit between the reader and the
 *  text Yomitan scans, and a mark that swallowed a long-press would cost a
 *  lookup to gain a judgement.
 *
 *  A tap on an unmarked word — a name, a non-word, anything the ledger will not
 *  hold — finds no span and does nothing, which is the intended silence rather
 *  than a case to report. */
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

/** Every occurrence of one term on screen, restatused. One word is one
 *  assertion: the same term three lines up carries the judgement just made, and
 *  leaving it marked would read as a write that failed. */
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
