// Vocabulary triage: turn untriaged terms into assertions.
//
// This is spec/cold-start.md's Pass 2 over the words already in the ledger, and
// the first thing in the workspace that writes `status` at all. Everything
// downstream — the #read highlighter, i+1 filtering, the vocabulary count —
// reduces to a status lookup, so until this has been run they all read as
// "nothing is known".
//
// The interaction is a checked sweep, not a decision per word. A ticked box
// means known, an unticked one means unknown, and submitting writes both. That
// is only safe because of what gets ticked *for* you: the server preselects a
// word only if it was met at least `min_encounters` times AND was never looked
// up (see jp_core::knowledge::vocabulary::preselects_known). Encounters alone
// would tick words you skimmed past; the zero-lookup half is what makes the
// default defensible.
//
// Two deliberate frictions:
//
//   - **The batch is what is on screen.** Submitting judges these rows and no
//     others. A word further down the queue than this page is left `new`, so a
//     half-finished pass leaves a queue you can resume rather than a ledger of
//     guesses.
//   - **The threshold is previewable.** Changing it re-queries rather than
//     re-filtering locally, so the count you see is the count the server would
//     act on. It is saved separately, in Settings.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

export function TriageView({ minEncounters, onJudged }) {
  const [queue, setQueue] = useState(null);
  const [floor, setFloor] = useState(minEncounters);
  // headword\treading → true (known) / false (unknown). Seeded from the
  // server's preselect, then owned by the reader.
  const [checked, setChecked] = useState({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);
  const [done, setDone] = useState(null);

  async function load(min) {
    setErr(null);
    try {
      const q = await api(`/api/vocab/queue?min_encounters=${min}`);
      setQueue(q);
      const seed = {};
      for (const t of q.terms) seed[key(t)] = t.preselect;
      setChecked(seed);
    } catch (e) {
      setErr(e.message);
    }
  }

  useEffect(() => {
    load(floor);
  }, [floor]);

  async function submit() {
    setBusy(true);
    setErr(null);
    try {
      const judgements = queue.terms.map((t) => ({
        headword: t.headword,
        reading: t.reading,
        status: checked[key(t)] ? "known" : "unknown",
      }));
      const res = await api("/api/vocab/judge", {
        method: "POST",
        body: { judgements },
      });
      setDone(`${res.written} judged`);
      await load(floor);
      onJudged?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  async function blacklistNoise() {
    setBusy(true);
    setErr(null);
    try {
      const res = await api("/api/vocab/blacklist-non-words", {
        method: "POST",
        body: {},
      });
      setDone(`${res.blacklisted} blacklisted`);
      onJudged?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  if (err) return html`<p class="chart-empty">Failed: ${err}</p>`;
  if (!queue) return html`<p class="chart-empty">Loading…</p>`;

  const known = queue.terms.filter((t) => checked[key(t)]).length;
  const unknown = queue.terms.length - known;
  // One string, not markup: htm collapses whitespace at a line break, so text
  // and ${...} must not straddle one (see CLAUDE.md).
  const pendingLine = `${queue.pending.toLocaleString("en")} words await a verdict at this floor · ${queue.pending_preselected.toLocaleString("en")} of them never looked up`;
  const batchLine = `this page: ${known} known · ${unknown} unknown`;

  return html`
    <div class="card">
      <h2>triage</h2>
      <p class="meta-hint">${pendingLine}</p>
      <label class="triage-floor">
        seen at least
        <input
          type="number"
          min="1"
          value=${floor}
          onChange=${(e) => setFloor(Math.max(1, Number(e.target.value) || 1))}
        />
        times
      </label>
      <p class="meta-hint">
        Ticked means known. Only words never looked up are ticked for you —
        needing the dictionary once is enough to leave a word unticked. Submit
        writes both verdicts for the rows below, and nothing else.
      </p>
    </div>

    ${
      queue.terms.length === 0
        ? html`<div class="card">
            <p class="chart-empty">
              Nothing left to judge at this floor. Lower it to reach further
              down the tail.
            </p>
          </div>`
        : html`
            <div class="card">
              <div class="triage-actions">
                <span>${batchLine}</span>
                <span>
                  <button
                    class="pause-btn"
                    onClick=${() =>
                      setChecked(allTo(queue.terms, true))}
                    disabled=${busy}
                  >
                    all known
                  </button>
                  <button
                    class="pause-btn"
                    onClick=${() =>
                      setChecked(allTo(queue.terms, false))}
                    disabled=${busy}
                  >
                    none
                  </button>
                  <button class="pause-btn" onClick=${submit} disabled=${busy}>
                    ${busy ? "saving…" : "submit"}
                  </button>
                </span>
              </div>
              ${done && html`<p class="meta-hint">${done}</p>`}
              <table class="days triage-table">
                <thead>
                  <tr>
                    <th>known</th>
                    <th>word</th>
                    <th>reading</th>
                    <th>seen</th>
                    <th>looked up</th>
                  </tr>
                </thead>
                <tbody>
                  ${queue.terms.map(
                    (t) => html`
                      <tr key=${key(t)}>
                        <td>
                          <input
                            type="checkbox"
                            checked=${!!checked[key(t)]}
                            onChange=${(e) =>
                              setChecked((c) => ({
                                ...c,
                                [key(t)]: e.target.checked,
                              }))}
                          />
                        </td>
                        <td class="triage-word">
                          ${t.headword}${
                            t.mined
                              ? html`<span
                                  class="triage-mined"
                                  title="This word is in the Anki deck"
                                  >carded</span
                                >`
                              : null
                          }
                        </td>
                        <td>${t.reading || "—"}</td>
                        <td>${t.encounter_count.toLocaleString("en")}</td>
                        <td>${t.lookup_count || "—"}</td>
                      </tr>
                    `,
                  )}
                </tbody>
              </table>
            </div>
          `
    }

    <div class="card">
      <h2>the non-vocabulary tail</h2>
      <p class="meta-hint">
        Rows no loaded dictionary recognises as a word — tokenizer noise like
        っっ and あああ. The queue above never offers them; this clears them out
        so the untriaged count means "vocabulary still to judge".
      </p>
      <button class="pause-btn" onClick=${blacklistNoise} disabled=${busy}>
        blacklist them
      </button>
    </div>
  `;
}

/** The ledger's key, as a string — (headword, reading), never headword alone. */
function key(t) {
  return `${t.headword}\t${t.reading}`;
}

function allTo(terms, value) {
  const next = {};
  for (const t of terms) next[key(t)] = value;
  return next;
}
