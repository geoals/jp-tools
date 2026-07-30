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

/** Kept in the DOM at once. Enough to scroll back over a scene, small enough
 *  that a long session doesn't grow the page without bound. */
const MAX_LINES = 300;
const FONT_KEY = "reader-font-px";
const FONT_DEFAULT = 20;
const HIGHLIGHT_KEY = "reader-highlight";
/** The tiers the server sends. Anything else is ignored rather than drawn in a
 *  default colour — an unrecognised status is a version skew, not a word. */
const TIERS = ["seen", "new", "unknown"];
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

export function Reader() {
  const [lines, setLines] = useState([]);
  const [live, setLive] = useState(false);
  const [state, setState] = useState(null);
  const [clearing, setClearing] = useState(false);
  const [explaining, setExplaining] = useState(false);
  const [explain, setExplain] = useState(null);
  const [toast, setToast] = useState(null);
  const [fontPx, setFontPx] = useState(
    () => Number(localStorage.getItem(FONT_KEY)) || FONT_DEFAULT,
  );
  // Highlighting is on unless it was turned off, and the browser can paint it.
  // A browser without the Highlight API isn't offered the toggle at all rather
  // than being given one that does nothing.
  const [highlightOn, setHighlightOn] = useState(
    () => localStorage.getItem(HIGHLIGHT_KEY) !== "off",
  );
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
        return [...prev, line].slice(-MAX_LINES);
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

  // Re-pin to the bottom on a new line, and also whenever the explain panel
  // opens, fills in, or closes — each resizes the lines pane, and without this
  // the newest line would slip out of view above the panel instead of sitting
  // right on top of it. Respects a manual scroll-up (stick=false) either way.
  useEffect(() => {
    if (stick.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [lines, explain, explaining]);

  // Repaint whenever the text could have moved: a new line, a font change, the
  // toggle. Layout effect rather than effect — this measures nodes that were
  // just committed, and a frame with the text drawn but not yet marked is a
  // flash on every single line.
  useLayoutEffect(() => {
    paintMarks(
      lines,
      lineEls.current,
      highlightOn,
      listRef.current,
      marksRef.current,
    );
  }, [lines, highlightOn, fontPx]);

  // The other way the text reflows: the window, or the split pane beside the
  // VN, changing width. Nothing re-renders then, so nothing above would fire.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() =>
      paintMarks(lines, lineEls.current, highlightOn, el, marksRef.current),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, [lines, highlightOn, fontPx]);

  useEffect(() => {
    // Successes clear themselves; failures stay until the next attempt.
    if (!toast || !toast.ok) return;
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
  }

  function toggleHighlight() {
    setHighlightOn((on) => {
      localStorage.setItem(HIGHLIGHT_KEY, on ? "off" : "on");
      return !on;
    });
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
        return [...byId.values()].sort((a, b) => a.id - b.id).slice(-MAX_LINES);
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
  const emptyLabel = live
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
      <div
        class="reader-lines"
        ref=${listRef}
        onScroll=${onScroll}
        style=${`font-size: ${fontPx}px`}
      >
        <div class="reader-marks" ref=${marksRef} aria-hidden="true"></div>
        ${lines.length === 0 && html`<p class="reader-empty">${emptyLabel}</p>`}
        ${lines.map((l) => renderLine(l, lineEls.current))}
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
      ${
        toast &&
        html`<div class="reader-toast ${toast.ok ? "ok" : "err"}">
          <span>${toast.text}</span>
          ${
            toast.undo &&
            html`<button class="reader-undo" onClick=${undoClear}>undo</button>`
          }
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
          disabled=${clearing || lines.length === 0}
          onClick=${clearLast}
          title="Drop the newest line from the stats — lines hooked while finding the route, or a stretch re-read after skipping back"
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
 *  `null` as a row leaves, which is what keeps the map from growing past the
 *  MAX_LINES on screen. */
function keepLineEl(els, id, el) {
  if (el) els.set(id, el);
  else els.delete(id);
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
        if (!TIERS.includes(t.status) || t.start + t.len > node.length)
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
