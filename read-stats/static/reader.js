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
/** How often the work title / pause state are re-read. Slow enough to be free,
 *  fast enough that switching works on the dashboard lands before the next
 *  card is mined. */
const STATE_POLL_MS = 20_000;

export function Reader() {
  const [lines, setLines] = useState([]);
  const [live, setLive] = useState(false);
  const [state, setState] = useState(null);
  const [mining, setMining] = useState(false);
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

  useEffect(() => {
    if (stick.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [lines]);

  useEffect(() => {
    // Successes clear themselves; failures stay until the next attempt.
    if (!toast || !toast.ok) return;
    const t = setTimeout(() => setToast(null), TOAST_MS);
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
  const emptyLabel = live
    ? "Waiting for the next hooked line…"
    : "Not connected — is read-stats reachable?";
  const captureOff = state && state.capture_available === false;
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
      ${toast &&
      html`<div class="reader-toast ${toast.ok ? "ok" : "err"}">
        ${toast.text}
      </div>`}
      <div class="reader-actions">
        <button
          class="reader-pause ${paused ? "paused" : ""}"
          onClick=${togglePause}
        >
          ${pauseLabel}
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

/** "2.4s audio + screenshot attached · ✂" — the trim note only when there is one. */
function successText(r) {
  const base = `${r.duration}s audio + screenshot attached`;
  return r.note ? `${base} · ${r.note}` : base;
}
