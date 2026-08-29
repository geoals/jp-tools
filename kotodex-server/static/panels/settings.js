// The settings view: every knob that used to be a constant, in one place.
//
// Two kinds of thing live here, and the split is deliberate:
//
//   - **server settings** — the derivation thresholds and the goal. These are
//     rows in `settings`, they change what every number on the dashboard means,
//     and the whole history re-derives under the new value on the next request.
//     That is the point of them being settings at all.
//   - **this browser** — the theme. Not a row anywhere: it is a property of the
//     device you are looking at, and a device reading in the dark should not
//     have to agree with one that isn't.
//
// **A knob appears when the data it acts on exists.** A threshold on a ledger
// with no rows in it, or a page size for a book nobody has logged, is a question
// with no answer yet and a reason to close the page. `advanced` is the rest: the
// derivation thresholds are real and reversible, and they are also nine numbers
// with a paragraph each, which is not what a first visit should open on.
//
// The Anki import is here for the same reason as the tokenizer link: it is a
// maintenance action on the ledger, run once in a while, not a reading of it.
// The vocab tab reports what the ledger holds.
//
// What is deliberately *not* here: the current work and the VN window. Both are
// per-work workflow rather than configuration, and they belong next to the work
// they describe (Currently reading, Library) where the context is.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";
import { THEMES, setTheme, storedTheme } from "../lib/theme.js";
import { AiSettings } from "./ai-settings.js";

/**
 * One numeric setting. `step` and `unit` are presentation; `hint` is the part
 * that matters — a threshold nobody can explain is a threshold nobody should
 * be turning.
 *
 * `min` must be a multiple of `step`: the browser validates values as
 * `min + n*step`, so `min: 1, step: 5` silently refuses to save 60.
 */
const FIELDS = [
  {
    group: "Goal",
    key: "goal_target_mins",
    label: "Daily target",
    unit: "minutes",
    step: 5,
    min: 5,
    hint: "Minutes a day that count as meeting the goal.",
  },
  {
    group: "Goal",
    key: "streak_min_mins",
    label: "Streak minimum",
    unit: "minutes",
    step: 5,
    min: 5,
    hint: "Minutes a day needs to extend the streak.",
  },
  {
    group: "Derivation",
    key: "afk_secs",
    label: "Gap cap",
    unit: "seconds",
    step: 5,
    min: 5,
    hint: "The most one gap between lines can count as reading.",
  },
  {
    group: "Derivation",
    key: "session_gap_secs",
    label: "Session break",
    unit: "seconds",
    step: 60,
    min: 60,
    hint: "A gap longer than this ends one sitting and starts another.",
  },
  {
    group: "Derivation",
    key: "day_rollover_hour",
    label: "Day starts at",
    unit: "o'clock",
    step: 1,
    min: 0,
    max: 23,
    hint: "Reading past midnight counts toward the day before, up to this hour.",
  },
  {
    group: "Derivation",
    key: "chars_per_page",
    label: "Characters per page",
    unit: "chars",
    step: 10,
    min: 10,
    hint: "For turning a book's page count into characters.",
  },
  {
    group: "Vocabulary",
    key: "triage_min_encounters",
    label: "Triage floor",
    unit: "encounters",
    step: 1,
    min: 1,
    hint: "How many times a word must appear before triage offers it. Only words never looked up are marked.",
  },
  {
    group: "Vocabulary",
    key: "reader_common_max_freq_rank",
    label: "Common word rank",
    unit: "jiten rank",
    step: 500,
    min: 0,
    hint: "New or unknown words ranked this high or higher are underlined in the reader.",
  },
  {
    group: "Vocabulary",
    key: "reader_common_max_bccwj_rank",
    label: "Common word rank",
    unit: "BCCWJ rank",
    step: 1000,
    min: 0,
    hint: "Same underline against BCCWJ. A word passing either threshold is underlined. 0 turns this off.",
  },
];

/** Shown straight away. Everything else is a threshold on data that has to
 *  exist first, and reads as a wall of numbers before it does. */
const GROUPS = ["Goal"];

/** Folded away, in this order. `Vocabulary` is dropped entirely while the ledger
 *  is empty: a triage floor with nothing to triage is a question with no answer
 *  yet. */
const ADVANCED_GROUPS = ["Derivation", "Vocabulary"];

export function SettingsView({ settings, vocab, onSaved }) {
  // Edits are staged locally and saved as a batch: several of these interact
  // (target and streak minimum, gap cap and session break), and saving on every
  // keystroke would re-derive the whole history per digit typed.
  const [draft, setDraft] = useState({});
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [err, setErr] = useState(null);
  const [theme, setThemeState] = useState(storedTheme);

  const valueOf = (key) => draft[key] ?? settings[key];
  const dirty = FIELDS.some(
    (f) => draft[f.key] !== undefined && Number(draft[f.key]) !== settings[f.key],
  );

  function edit(key, raw) {
    setSaved(false);
    setDraft((d) => ({ ...d, [key]: raw }));
  }

  function pickTheme(next) {
    setTheme(next);
    setThemeState(next);
  }

  async function save(e) {
    e.preventDefault();
    const body = {};
    for (const f of FIELDS) {
      if (draft[f.key] === undefined) continue;
      const n = Number(draft[f.key]);
      if (!Number.isFinite(n)) {
        setErr(`${f.label} must be a number`);
        return;
      }
      if (n !== settings[f.key]) body[f.key] = n;
    }
    if (!Object.keys(body).length) return;
    setBusy(true);
    setErr(null);
    try {
      await api("/api/settings", { method: "PUT", body });
      setDraft({});
      setSaved(true);
      onSaved();
    } catch (e2) {
      setErr(e2.message);
    } finally {
      setBusy(false);
    }
  }

  function reset() {
    setDraft({});
    setErr(null);
    setSaved(false);
  }

  const saveLabel = busy ? "saving…" : "save changes";

  // The ledger fills itself from what has been read, so this is "have you read
  // anything yet" as much as it is about vocabulary.
  const hasLedger = (vocab?.total ?? 0) > 0;
  const advanced = ADVANCED_GROUPS.filter((g) => g !== "Vocabulary" || hasLedger);

  const group = (name) => html`
    <div class="settings-group" key=${name}>
      <h3>${name}</h3>
      ${FIELDS.filter((f) => f.group === name).map(
        (f) => html`
          <div class="settings-row" key=${f.key}>
            <label for=${`set-${f.key}`}>${f.label}</label>
            <div class="settings-input">
              <input
                id=${`set-${f.key}`}
                type="number"
                step=${f.step}
                min=${f.min}
                max=${f.max}
                value=${valueOf(f.key)}
                onInput=${(e) => edit(f.key, e.currentTarget.value)}
              />
              <span class="settings-unit">${f.unit}</span>
            </div>
            ${f.hint && html`<p class="settings-hint">${f.hint}</p>`}
          </div>
        `,
      )}
    </div>
  `;

  return html`
    <div class="card">
      <h2
        title="Every threshold is applied when reading the data, not when capturing it — changing one recalculates your whole history, and changing it back undoes that exactly."
      >
        Settings
      </h2>
      <${AiSettings} settings=${settings} onSaved=${onSaved} />
      <form onSubmit=${save}>
        ${GROUPS.map(group)}
        <details class="settings-advanced">
          <summary>Advanced — how reading is measured</summary>
          <p class="settings-hint">
            Changes recalculate your whole history. Changing back undoes it.
          </p>
          ${advanced.map(group)}
        </details>
        <div class="settings-actions">
          <button type="submit" disabled=${busy || !dirty}>${saveLabel}</button>
          ${dirty &&
          html`<button type="button" class="ghost" onClick=${reset}>
            discard
          </button>`}
          ${saved && !dirty && html`<span class="goal-met">saved</span>`}
          ${err && html`<span class="settings-err">${err}</span>`}
        </div>
      </form>

      <div class="settings-group">
        <h3>This browser</h3>
        <div class="settings-row">
          <label>Theme</label>
          <div class="settings-input">
            <div class="radio-set" role="radiogroup" aria-label="Theme">
              ${THEMES.map(
                (t) => html`
                  <label class="radio-opt" key=${t}>
                    <input
                      type="radio"
                      name="theme"
                      value=${t}
                      checked=${theme === t}
                      onChange=${() => pickTheme(t)}
                    />
                    <span>${t}</span>
                  </label>
                `,
              )}
            </div>
          </div>
        </div>
      </div>

      <div class="settings-group">
        <h3>Tools</h3>
        <div class="settings-row">
          <label>Setup</label>
          <div class="settings-input">
            <a class="pause-btn" href="#setup">✓ what is set up</a>
          </div>
        </div>
        <div class="settings-row">
          <label>Anki import</label>
          <div class="settings-input">
            <${AnkiImport} onImported=${onSaved} />
          </div>
          <p class="settings-hint">
            Cards past Anki's new/learning queues are marked known. Never
            overwrites a word already judged.
          </p>
        </div>
        <div class="settings-row">
          <label>Reader</label>
          <div class="settings-input">
            <a class="pause-btn" href="#read">📖 open in a browser</a>
          </div>
        </div>
        <div class="settings-row">
          <label>Tokenizer</label>
          <div class="settings-input">
            <a class="pause-btn" href="#tokenize">🔤 tokenize a line</a>
          </div>
        </div>
      </div>
    </div>
  `;
}

/** The one-shot Anki import: a card the reader is reviewing is evidence enough
 *  to mark its word known without asking again. */
function AnkiImport({ onImported }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState(null);
  const [err, setErr] = useState(null);

  async function run() {
    setBusy(true);
    setErr(null);
    try {
      const res = await api("/api/vocab/anki-import", { method: "POST" });
      const skipped = res.ambiguous_skipped
        ? ` · ${res.ambiguous_skipped} skipped (more than one reading)`
        : "";
      setResult(`${res.imported} marked known${skipped}`);
      onImported?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  return html`
    <button type="button" class="pause-btn" onClick=${run} disabled=${busy}>
      ${busy ? "importing…" : "import reviewing cards"}
    </button>
    ${err && html`<span class="settings-err">${err}</span>`}
    ${result && html`<span class="settings-hint">${result}</span>`}
  `;
}
