// Vocabulary triage: turn untriaged terms into assertions.
//
// The first thing in the workspace that writes `status` at all — everything
// downstream reduces to a status lookup, so until this has run they all read as
// "nothing is known".
//
// The batch defaults to words read *since the last sweep*, so a day or a
// fortnight of reading produces a short list already ticked. The standing
// backlog is one toggle away: the watermark narrows what is asked about and
// judges nothing itself, and it moves on submit rather than on load, so an
// interrupted sweep keeps its batch.
//
// The interaction is a checked sweep, not a decision per word: ticked means
// known, unticked means unknown, and submitting writes both. That is only safe
// because of what gets ticked *for* you — the server preselects a word only if
// it was met at least `min_encounters` times AND was never looked up
// (`vocabulary::preselects_known`). The zero-lookup half is what makes the
// default defensible.
//
// Two deliberate frictions:
//
//   - **The batch is what is on screen**, so a half-finished pass leaves a
//     resumable queue rather than a ledger of guesses.
//   - **The threshold is previewable.** Changing it re-queries rather than
//     filtering locally, so the count shown is the count the server would act
//     on. It is saved separately, in Settings.
//
// The same rule covers the non-vocabulary tail below.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";
import { fmtTsDate } from "../lib/format.js";

export function TriageView({ minEncounters, onJudged }) {
  const [queue, setQueue] = useState(null);
  const [floor, setFloor] = useState(minEncounters);
  // The sweep's scoping. On by default: the standing backlog is the exception
  // now, not the daily loop.
  const [scoped, setScoped] = useState(true);
  // headword\treading → true (known) / false (unknown). Seeded from the
  // server's preselect, then owned by the reader.
  const [checked, setChecked] = useState({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);
  const [done, setDone] = useState(null);
  // The non-vocabulary tail, once asked for. Never fetched with the queue: it
  // is a separate question, and one nobody asks on every visit.
  const [noise, setNoise] = useState(null);

  async function load(min, isScoped) {
    setErr(null);
    try {
      const q = await api(
        `/api/vocab/queue?min_encounters=${min}&scoped=${isScoped ? 1 : 0}`,
      );
      setQueue(q);
      const seed = {};
      for (const t of q.terms) seed[key(t)] = t.preselect;
      setChecked(seed);
    } catch (e) {
      setErr(e.message);
    }
  }

  useEffect(() => {
    load(floor, scoped);
  }, [floor, scoped]);

  async function submit() {
    setBusy(true);
    setErr(null);
    try {
      const judgements = queue.terms.map((t) => ({
        headword: t.headword,
        reading: t.reading,
        status: checked[key(t)] ? "known" : "unknown",
      }));
      // The watermark moves only for a batch that was actually scoped to it.
      // Submitting from the full backlog is a one-off judgement, and retiring
      // a sweep nobody was shown would lose words to it.
      const res = await api("/api/vocab/judge", {
        method: "POST",
        body: { judgements, advance_sweep: queue.scoped },
      });
      setDone(`${res.written} judged`);
      await load(floor, scoped);
      onJudged?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  async function showNoise(offset = 0) {
    setErr(null);
    try {
      setNoise(await api(`/api/vocab/non-words?offset=${offset}`));
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
  const sweptOn = fmtTsDate(queue.since);
  // One string, not markup: htm collapses whitespace at a line break, so text
  // and ${...} must not straddle one (see CLAUDE.md).
  const scopeLine = queue.scoped
    ? `read since the last sweep${sweptOn ? ` on ${sweptOn}` : ""}`
    : "everything ready, however long ago it was read";
  const pendingLine = `${queue.pending.toLocaleString("en")} words await a verdict at this floor · ${queue.pending_preselected.toLocaleString("en")} of them never looked up · ${scopeLine}`;
  const batchLine = `this page: ${known} known · ${unknown} unknown`;
  const acceptLabel = `accept batch (${known} known · ${unknown} unknown)`;
  const emptyLine = queue.scoped
    ? "Nothing new since the last sweep. Come back after a day of reading, or show the whole backlog."
    : "Nothing left to judge at this floor. Lower it to reach further down the tail.";

  return html`
    <div class="card">
      <h2>sweep</h2>
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
      <label class="triage-floor">
        <input
          type="checkbox"
          checked=${!scoped}
          onChange=${(e) => setScoped(!e.target.checked)}
        />
        show everything ready, not just since the last sweep
      </label>
      <p class="meta-hint">
        Ticked means known. Only words never looked up are ticked for you —
        needing the dictionary once is enough to leave a word unticked.
        Accepting writes both verdicts for the rows below and nothing else:
        unticked means unknown, which is the snooze that keeps a word out of
        every later batch until you read it a lot more.
      </p>
    </div>

    ${
      queue.terms.length === 0
        ? html`<div class="card">
            <p class="chart-empty">${emptyLine}</p>
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
                    ${busy ? "saving…" : acceptLabel}
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
          ? html`<button class="ghost" onClick=${() => showNoise(0)}>
              show me what they are
            </button>`
          : html`<${NoisePreview}
              noise=${noise}
              busy=${busy}
              onPage=${showNoise}
              onBlacklist=${blacklistNoise}
              onCancel=${() => setNoise(null)}
            />`
      }
    </div>
  `;
}

/** What the bulk write would hit, before it hits it.
 *
 *  The words come first and the button second: this is the one action here that
 *  judges rows the reader has not seen, and a count alone cannot be checked.
 *
 *  A table rather than chips, because the encounter count is half the evidence —
 *  a real word wrongly in this set is one with sightings behind it. Paged rather
 *  than truncated for the same reason: the head being noise says nothing about
 *  the tail. */
function NoisePreview({ noise, busy, onPage, onBlacklist, onCancel }) {
  if (!noise.total) {
    return html`<p class="meta-hint">Nothing in the tail — already clear.</p>`;
  }
  const { total, offset, limit, terms } = noise;
  const last = Math.min(offset + terms.length, total);
  const rangeLine = `${(offset + 1).toLocaleString("en")}–${last.toLocaleString("en")} of ${total.toLocaleString("en")}, commonest first`;

  return html`
    <p class="meta-hint">${rangeLine}</p>
    <table class="days noise-table">
      <thead>
        <tr>
          <th>word</th>
          <th>reading</th>
          <th>pos</th>
          <th>seen</th>
        </tr>
      </thead>
      <tbody>
        ${terms.map(
          (t) => html`
            <tr>
              <td class="work-name">${t.headword}</td>
              <td class="work-name">${t.reading}</td>
              <td class="work-name">${t.pos ?? "—"}</td>
              <td>${t.encounter_count.toLocaleString("en")}</td>
            </tr>
          `,
        )}
      </tbody>
    </table>
    <div class="triage-actions">
      <span>
        <button
          class="ghost"
          disabled=${offset === 0 || busy}
          onClick=${() => onPage(Math.max(0, offset - limit))}
        >
          ← previous
        </button>
        <button
          class="ghost"
          disabled=${last >= total || busy}
          onClick=${() => onPage(offset + limit)}
        >
          next →
        </button>
      </span>
      <span>
        <button class="pause-btn" onClick=${onBlacklist} disabled=${busy}>
          ${busy ? "…" : `blacklist all ${total.toLocaleString("en")}`}
        </button>
        <button class="ghost" onClick=${onCancel} disabled=${busy}>
          cancel
        </button>
      </span>
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
