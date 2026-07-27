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
//
// The same rule covers the non-vocabulary tail below: it is a bulk write over
// rows the queue never shows, so the words go on screen first and the button
// only appears once they have.

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
  // The non-vocabulary tail, once asked for. Never fetched with the queue: it
  // is a separate question, and one nobody asks on every visit.
  const [noise, setNoise] = useState(null);

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

  async function showNoise() {
    setErr(null);
    try {
      setNoise(await api("/api/vocab/non-words"));
    } catch (e) {
      setErr(e.message);
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
      setNoise(null);
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
                    onClick=${() => setChecked(allTo(queue.terms, true))}
                    disabled=${busy}
                  >
                    all known
                  </button>
                  <button
                    class="pause-btn"
                    onClick=${() => setChecked(allTo(queue.terms, false))}
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
        っっ and あああ. The queue above never offers them; blacklisting clears
        them out so the untriaged count means "vocabulary still to judge".
      </p>
      ${
        noise === null
          ? html`<button class="ghost" onClick=${showNoise}>
              show me what they are
            </button>`
          : html`<${NoisePreview}
              noise=${noise}
              busy=${busy}
              onBlacklist=${blacklistNoise}
              onCancel=${() => setNoise(null)}
            />`
      }
    </div>
  `;
}

/** What the bulk write would hit, before it hits it.
 *
 *  The words come first and the button second: this is the one action here
 *  that judges rows the reader has not seen, and a count alone ("3,140
 *  blacklisted") is not something anyone can check. Commonest first, because a
 *  real word wrongly in this list would be one with encounters behind it. */
function NoisePreview({ noise, busy, onBlacklist, onCancel }) {
  if (!noise.total) {
    return html`<p class="meta-hint">Nothing in the tail — already clear.</p>`;
  }
  const shownLine =
    noise.total > noise.shown
      ? `${noise.total.toLocaleString("en")} rows, commonest ${noise.shown} shown`
      : `${noise.total.toLocaleString("en")} rows, all shown`;
  return html`
    <p class="meta-hint">${shownLine}</p>
    <div class="word-chips">
      ${noise.terms.map(
        (t) =>
          html`<span class="chip"
            >${t.headword} <b>×${t.encounter_count}</b></span
          >`,
      )}
    </div>
    <div class="triage-actions">
      <button class="pause-btn" onClick=${onBlacklist} disabled=${busy}>
        ${busy ? "…" : `blacklist all ${noise.total.toLocaleString("en")}`}
      </button>
      <button class="ghost" onClick=${onCancel} disabled=${busy}>cancel</button>
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
