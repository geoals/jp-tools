/** Phone reading view: the live Textractor line feed, sized for a split-screen
 *  half next to a Moonlight stream, with the mine button that fires
 *  vn-capture.sh back on the PC.
 *
 *  Lines are plain text nodes on purpose — Yomitan scans the DOM, so anything
 *  clever here (virtualized rows, per-token spans) would break lookups. */
import { useEffect, useRef, useState } from "preact/hooks";
import { html } from "htm/preact";
import { api } from "./api.js";

/** Kept in the DOM at once. Enough to scroll back over a scene, small enough
 *  that a long session doesn't grow the page without bound. */
const MAX_LINES = 300;
const FONT_KEY = "reader-font-px";
const FONT_DEFAULT = 20;
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
  const [mining, setMining] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [explaining, setExplaining] = useState(false);
  const [explain, setExplain] = useState(null);
  const [toast, setToast] = useState(null);
  const [fontPx, setFontPx] = useState(
    () => Number(localStorage.getItem(FONT_KEY)) || FONT_DEFAULT,
  );
  const listRef = useRef(null);
  const stick = useRef(true);

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

  function bumpFont(delta) {
    setFontPx((px) => {
      const next = Math.min(40, Math.max(13, px + delta));
      localStorage.setItem(FONT_KEY, String(next));
      return next;
    });
  }

  async function mine() {
    setMining(true);
    setToast(null);
    try {
      const r = await api("/api/vn/capture", { method: "POST", body: {} });
      setToast(
        r.ok
          ? { ok: true, text: successText(r) }
          : { ok: false, text: r.error || "capture failed" },
      );
    } catch (err) {
      setToast({ ok: false, text: err.message });
    } finally {
      setMining(false);
    }
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
      const r = await api("/api/pause", { method: "POST", body: {} });
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
  const mineLabel = mining ? "mining…" : "⛏ mine last line";
  const pauseLabel = paused ? "▶ resume" : "⏸ pause";
  const clearLabel = clearing ? "…" : "✕ clear last";
  const emptyLabel = live
    ? "Waiting for the next hooked line…"
    : "Not connected — is read-stats reachable?";
  const captureOff = state && state.capture_available === false;
  const explainOff = state && state.explain_available === false;
  // Built whole rather than split around the focus word — htm collapses the
  // whitespace where literal text meets an interpolation across a line break.
  const explainTitle = explain && explain.focus
    ? `“${explain.focus}” in the last line`
    : "the last line";
  // Deliberately loud. Pause doesn't auto-resume (a skip-pause has to survive
  // lines flying past), so the only thing standing between a forgotten pause
  // and an evening of uncounted reading is noticing it.
  const pausedBanner = "⏸ PAUSED — nothing is counting. Tap to resume.";

  return html`
    <div class="reader ${paused ? "is-paused" : ""}">
      <div class="reader-bar">
        <a class="reader-back" href="#" title="Back to the dashboard">←</a>
        <span class="reader-work">${work}</span>
        <span class="reader-live ${live ? "on" : "off"}">${liveLabel}</span>
        <button class="ghost" onClick=${() => bumpFont(-2)}>A−</button>
        <button class="ghost" onClick=${() => bumpFont(2)}>A+</button>
      </div>
      ${paused &&
      html`<button class="reader-paused" onClick=${togglePause}>
        ${pausedBanner}
      </button>`}
      <div
        class="reader-lines"
        ref=${listRef}
        onScroll=${onScroll}
        style=${`font-size: ${fontPx}px`}
      >
        ${lines.length === 0 &&
        html`<p class="reader-empty">${emptyLabel}</p>`}
        ${lines.map(
          (l) => html`<p class="reader-line" key=${l.id}>${l.text}</p>`,
        )}
      </div>
      ${(explaining || explain) &&
      html`<div class="reader-explain ${explain && explain.ok === false ? "err" : ""}">
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
      </div>`}
      ${toast &&
      html`<div class="reader-toast ${toast.ok ? "ok" : "err"}">
        <span>${toast.text}</span>
        ${toast.undo &&
        html`<button class="reader-undo" onClick=${undoClear}>undo</button>`}
      </div>`}
      <div class="reader-actions">
        <button
          class="reader-pause ${paused ? "paused" : ""}"
          onClick=${togglePause}
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
          ${explaining ? "…" : "ℹ"}
        </button>
        <button
          class="reader-mine"
          disabled=${mining || captureOff}
          onClick=${mine}
          title="Attach the last voiceline's audio + a screenshot to the newest Anki note"
        >
          ${mineLabel}
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
          (l, j) => html`<li key=${j}>${inlineMd(l.replace(/^\s*[-*]\s+/, ""))}</li>`,
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

/** "2.4s audio + screenshot attached · ✂" — the trim note only when there is one. */
function successText(r) {
  const base = `${r.duration}s audio + screenshot attached`;
  return r.note ? `${base} · ${r.note}` : base;
}
