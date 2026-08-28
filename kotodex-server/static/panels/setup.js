// What has to happen before there is anything to read, one thing at a time.
//
// **This is not a wizard.** There is no step counter and no "finished" flag: the
// state is `GET /api/setup`, which is the live capability probe, so a part that
// breaks in six months shows the row it showed on the first run and there is
// nothing stored that can disagree with the machine.
//
// Blocking rows gate the dashboard (see `app.js`) and are shown **one at a
// time**, oldest obstacle first — the reader has one thing to do, so the page
// asks for one thing. Everything else is a short list underneath, phrased as
// what it would add rather than as a fault, because none of it stops reading.
//
// `check again` re-probes past the cache. The reader has just done something
// outside the app, and that is the worst moment for a stale answer.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

/** Reading order for the blocking rows: text has to arrive before a dictionary
 *  can be asked about it. */
const BLOCKING_ORDER = ["lines_source", "dict_definitions"];

/** The prose each blocking row needs and the probe cannot carry: what this part
 *  is, in the reader's terms, before the sentence saying it is missing. */
const HEADINGS = {
  lines_source: "Kotodex needs the game's text",
  dict_definitions: "Kotodex needs a dictionary",
};

/** The steps behind a blocking row, where there are steps rather than a sentence.
 *
 *  Client-side prose and not part of the probe: the server's `fix` is one
 *  sentence saying what is wrong, and this is a page telling someone what to do
 *  with their hands.
 *
 *  **The flush delay is here because it cannot be detected.** Textractor at its
 *  default 500 ms holds a line for half a second before releasing it, which is
 *  the overlay visibly lagging the game — but nothing server-side can measure it.
 *  The logger's line merge is content-based on purpose (`continues_previous`), so
 *  the one signal that looked like a proxy is not one, and a guess printed as a
 *  finding is worse than an instruction.
 */
const STEPS = {
  lines_source: [
    "Install Textractor and attach it to the game.",
    "In Textractor, open Extensions and add the WebSocket extension.",
    "Set Textractor's flush delay to 30 ms. It ships at 500, which makes every line appear half a second after the game draws it.",
    "Pick the hook that gives you the dialogue and nothing else.",
  ],
  dict_definitions: [
    "Download a Yomitan dictionary — Jitendex is a good first one, and free.",
    "Drop the zip into the dictionaries folder next to Kotodex.",
    "Restart Kotodex. It imports anything new as it starts.",
  ],
};

/** What a working install adds, for the rows that are not blocking. Keyed the
 *  same way the probe is; anything unlisted is shown by its key. */
const LABELS = {
  anki: "Anki, for mining cards",
  anki_note_type: "the note type cards are made on",
  explain: "AI explanations, and the gloss on a card",
  whisper: "trimming card audio to the mined sentence",
  vocabulary_ledger: "your vocabulary ledger",
  dict_master: "a master dictionary, for the vocabulary count",
  dict_frequency: "how common a word is",
  dict_pitch: "pitch accent",
  capture_running: "recording, for audio on a card",
  vad_model: "trimming silence off a clip",
  screenshot_tool: "a screenshot on a card",
  xdotool: "following the game's window",
  overlay_backend: "the overlay above the game",
};

export function SetupView({ onReady }) {
  const [caps, setCaps] = useState(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);

  async function probe() {
    setBusy(true);
    try {
      const next = await api("/api/setup");
      setCaps(next);
      setErr(null);
      // The reader is here because something was blocking. Telling the shell the
      // moment it is not is what takes them to the dashboard without a reload.
      if (!blocking(next).length) onReady?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    probe();
  }, []);

  if (err) return html`<div class="card"><p class="chart-empty">${err}</p></div>`;
  if (!caps) return html`<div class="card"><p class="chart-empty">Checking…</p></div>`;

  const blockers = blocking(caps);
  const now = blockers[0];
  const later = Object.entries(caps)
    .filter(([key, c]) => !c.ok && !c.blocking && key !== now)
    .sort(([a], [b]) => a.localeCompare(b));

  return html`
    <div class="card setup">
      ${now
        ? html`
            <h2>${HEADINGS[now] ?? now}</h2>
            <p class="setup-fix">${caps[now].fix}</p>
            ${STEPS[now] &&
            html`<ol class="setup-steps">
              ${STEPS[now].map((step) => html`<li key=${step}>${step}</li>`)}
            </ol>`}
            <p class="setup-detail">Right now: ${caps[now].detail}</p>
            ${blockers.length > 1 &&
            html`<p class="settings-hint">
              One more thing after this one.
            </p>`}
          `
        : html`
            <h2>Ready to read</h2>
            <p class="setup-fix">
              Text is arriving and a dictionary can answer for it. Everything
              below is optional — add it whenever you want it.
            </p>
          `}
      <div class="setup-actions">
        <button onClick=${probe} disabled=${busy}>
          ${busy ? "checking…" : "check again"}
        </button>
        ${caps[now]?.action &&
        html`<a class="pause-btn" href=${caps[now].action.goto}>
          ${caps[now].action.label}
        </a>`}
      </div>

      ${later.length > 0 &&
      html`
        <div class="settings-group">
          <h3>Not set up yet</h3>
          ${later.map(
            ([key, c]) => html`
              <div class="setup-row" key=${key}>
                <div class="setup-row-head">
                  <span class="setup-row-label">${LABELS[key] ?? key}</span>
                  ${c.action &&
                  html`<a class="ghost" href=${c.action.goto}>${c.action.label}</a>`}
                </div>
                <p class="settings-hint">${c.fix}</p>
              </div>
            `,
          )}
        </div>
      `}
    </div>
  `;
}

/** The blocking rows that are off, in the order they have to be dealt with. An
 *  unrecognised blocking key still shows, after the ones with an order. */
function blocking(caps) {
  return Object.entries(caps)
    .filter(([, c]) => c.blocking && !c.ok)
    .map(([key]) => key)
    .sort((a, b) => {
      const rank = (k) => {
        const i = BLOCKING_ORDER.indexOf(k);
        return i === -1 ? BLOCKING_ORDER.length : i;
      };
      return rank(a) - rank(b) || a.localeCompare(b);
    });
}

/** Whether the dashboard should be replaced by the setup view.
 *
 *  Exported so `app.js` can ask without rendering: the gate is one question and
 *  the answer has to be the same one this panel shows.
 */
export function isBlocked(caps) {
  return !!caps && blocking(caps).length > 0;
}
