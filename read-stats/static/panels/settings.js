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
    hint: "The one goal the meter draws, and what “target met” means. 120 is roughly two hours of reading a day.",
  },
  {
    group: "Goal",
    key: "streak_min_mins",
    label: "Streak minimum",
    unit: "minutes",
    step: 5,
    min: 5,
    hint: "Minutes a day needs to extend the streak. Separate from the target on purpose: the streak asks whether you showed up, the target asks whether you did the day's work.",
  },
  {
    group: "Derivation",
    key: "afk_secs",
    label: "Gap cap",
    unit: "seconds",
    step: 5,
    min: 5,
    hint: "The most one gap between lines can credit as reading. 30s is about the 90th percentile of real lookup gaps — long enough to keep a dictionary detour whole, short enough that walking away doesn't count. Changing this re-prices every hour you have ever read.",
  },
  {
    group: "Derivation",
    key: "session_gap_secs",
    label: "Session break",
    unit: "seconds",
    step: 60,
    min: 60,
    hint: "A gap longer than this ends one sitting and starts another. Only affects how reading is grouped, never how much of it counts.",
  },
  {
    group: "Derivation",
    key: "day_rollover_hour",
    label: "Day starts at",
    unit: "o'clock",
    step: 1,
    min: 0,
    max: 23,
    hint: "Reading past midnight counts toward the day before, up to this hour. 4 means a session ending at 03:30 still belongs to yesterday.",
  },
  {
    group: "Derivation",
    key: "chars_per_page",
    label: "Characters per page",
    unit: "chars",
    step: 10,
    min: 10,
    hint: "Used to turn a physical book's page count into characters when logging a session by hand. 550 is a typical bunkobon page.",
  },
  {
    group: "Vocabulary",
    key: "triage_min_encounters",
    label: "Triage floor",
    unit: "encounters",
    step: 1,
    min: 1,
    hint: "How often a word must have been met before triage offers it, and ticks it as known. It can sit this low because the other half of the rule does the work: only words you have never looked up are ticked. Raise it for a shorter, safer queue; lower it to reach further down the tail.",
  },
  {
    group: "Vocabulary",
    key: "reader_common_max_freq_rank",
    label: "Common word rank",
    unit: "jiten rank",
    step: 500,
    min: 0,
    hint: "In #read, a new or unknown word this common or commoner is underlined as well as tinted. Not knowing a rare word is expected; a common one is the gap worth seeing. Words the list does not rank are never underlined by this threshold.",
  },
  {
    group: "Vocabulary",
    key: "reader_common_max_bccwj_rank",
    label: "Common word rank",
    unit: "BCCWJ rank",
    step: 1000,
    min: 0,
    hint: "The same underline against BCCWJ, tested on its own: a word passing either threshold is underlined. BCCWJ is newspaper and government prose, so it catches common words the fiction list ranks rare. 0 turns this half off.",
  },
];

const GROUPS = ["Goal", "Derivation", "Vocabulary"];

export function SettingsView({ settings, onSaved }) {
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

  return html`
    <div class="card">
      <h2
        title="Thresholds are applied at query time, not at capture time — every one of these re-reads the whole history under the new value, so a change is reversible and never edits a stored number."
      >
        Settings
      </h2>
      <form onSubmit=${save}>
        ${GROUPS.map(
          (group) => html`
            <div class="settings-group" key=${group}>
              <h3>${group}</h3>
              ${FIELDS.filter((f) => f.group === group).map(
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
                    <p class="settings-hint">${f.hint}</p>
                  </div>
                `,
              )}
            </div>
          `,
        )}
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
          <p class="settings-hint">
            Stored on this device only, so one device can read dark while
            another stays light. “system” follows the OS setting as it changes.
          </p>
        </div>
      </div>

      <div class="settings-group">
        <h3>Tools</h3>
        <div class="settings-row">
          <label>Anki import</label>
          <div class="settings-input">
            <${AnkiImport} onImported=${onSaved} />
          </div>
          <p class="settings-hint">
            Cards past Anki's new/learning queues are marked known outright.
            Never overwrites a word already judged here.
          </p>
        </div>
        <div class="settings-row">
          <label>Reading view</label>
          <div class="settings-input">
            <a class="pause-btn" href="#read">📖 open #read</a>
          </div>
          <p class="settings-hint">
            The live line feed in a browser, with Yomitan over it. The overlay
            is the everyday surface; this is for text no hook produces, another
            device, or going back over a session's lines.
          </p>
        </div>
        <div class="settings-row">
          <label>Tokenizer</label>
          <div class="settings-input">
            <a class="pause-btn" href="#tokenize">🔤 tokenize a line</a>
          </div>
          <p class="settings-hint">
            Paste a line and see what the pipeline made of it — the tint the
            reading view would paint, and one row per token.
          </p>
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
