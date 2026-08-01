// Frequency-threshold triage, over words the
// reading history has never even produced yet.
//
// Deliberately not a swipe UI. `triage.js` judges the ledger's own rows one
// screen at a time because each of those needs a real per-word verdict; the
// point of this pass is the opposite — a single rank threshold that marks
// thousands of words `known` in one click, because "I know most words this
// common" is one judgement, not one per word. The preview-then-commit shape is
// the same one `NoisePreview` already uses for the non-word blacklist: the
// words go on screen before the bulk write, never after.
//
// A word with more than one master-dictionary reading never reaches this
// list at all — the server excludes it before paging, not just before
// committing, so a page here is never spent on a row nothing can be done
// with. `vocab.frequency_pending`'s `ambiguous` count is where that total
// still shows up.
//
// A row can be marked "not known" with one click, same idea as triage.js's
// checkbox but without a submit step: it writes straight through the ledger's
// ordinary judge endpoint (`POST /api/vocab/judge`), and the row disappears
// on its own next fetch because the server-side query already excludes
// anything no longer `status = 'new'` — no frequency-specific "exclude" logic
// needed, only an optimistic removal from what's on screen already.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

export function FrequencyView({ maxFreqRank, onCommitted }) {
  const [rank, setRank] = useState(maxFreqRank);
  // { committable, ambiguous } — kept as two numbers rather than a total.
  // Ambiguous terms never get a ledger row, so they stay pending forever at
  // this threshold: a "mark all N known" button built from one combined
  // count would eventually promise work a second commit could not do.
  const [summary, setSummary] = useState(null);
  const [preview, setPreview] = useState(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);
  const [done, setDone] = useState(null);

  async function loadSummary(r) {
    setErr(null);
    try {
      const s = await api(`/api/vocab/frequency-summary?max_rank=${r}`);
      setSummary(s);
    } catch (e) {
      setErr(e.message);
    }
  }

  useEffect(() => {
    setPreview(null);
    setDone(null);
    loadSummary(rank);
  }, [rank]);

  async function showPreview(offset = 0) {
    setErr(null);
    try {
      setPreview(
        await api(`/api/vocab/frequency-queue?max_rank=${rank}&offset=${offset}`),
      );
    } catch (e) {
      setErr(e.message);
    }
  }

  async function excludeWord(term, reading) {
    // Optimistic: the row leaves the screen immediately, since waiting on the
    // round trip for a single-word "no" is the friction this exists to
    // remove. On failure it's put back and the error shown — better than a
    // silent word that looks handled but wasn't.
    const prevPreview = preview;
    const prevSummary = summary;
    setPreview((p) => p && removeTerm(p, term));
    setSummary((s) => s && { ...s, committable: Math.max(0, s.committable - 1) });
    try {
      await api("/api/vocab/judge", {
        method: "POST",
        body: { judgements: [{ headword: term, reading, status: "unknown" }] },
      });
    } catch (e) {
      setErr(e.message);
      setPreview(prevPreview);
      setSummary(prevSummary);
    }
  }

  async function commit() {
    setBusy(true);
    setErr(null);
    try {
      const res = await api("/api/vocab/frequency-commit", {
        method: "POST",
        body: { max_rank: rank },
      });
      const skippedLine = res.ambiguous_skipped
        ? ` · ${res.ambiguous_skipped} skipped (more than one reading)`
        : "";
      setDone(`${res.written} marked known${skippedLine}`);
      setPreview(null);
      await loadSummary(rank);
      onCommitted?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  if (err) return html`<p class="chart-empty">Failed: ${err}</p>`;

  return html`
    <div class="card">
      <h2>frequency triage</h2>
      <p class="meta-hint">
        Words this common are marked known in one sweep, on the assumption
        that a word inside the threshold is one you already have. Words with
        more than one dictionary reading are left for ordinary triage instead
        of guessed at.
      </p>
      <label class="triage-floor">
        known up to BCCWJ rank
        <input
          type="number"
          min="1"
          step="500"
          value=${rank}
          onChange=${(e) => setRank(Math.max(1, Number(e.target.value) || 1))}
        />
      </label>
      ${summary === null
        ? html`<p class="meta-hint">Loading…</p>`
        : html`<p class="meta-hint">
            ${summaryLine(summary)}
          </p>`}
      ${done && html`<p class="meta-hint">${done}</p>`}
      ${
        preview === null
          ? html`<button
              class="ghost"
              onClick=${() => showPreview(0)}
              disabled=${!summary || (!summary.committable && !summary.ambiguous)}
            >
              show me what they are
            </button>`
          : html`<${FrequencyPreview}
              preview=${preview}
              busy=${busy}
              onPage=${showPreview}
              onCommit=${commit}
              onCancel=${() => setPreview(null)}
              onExclude=${excludeWord}
            />`
      }
    </div>
  `;
}

/** One string, not markup — text and `${...}` must not straddle a line break
 *  inside an `html` template (see CLAUDE.md). */
function summaryLine(s) {
  if (!s.committable && !s.ambiguous) return "Nothing pending at this rank.";
  if (!s.ambiguous) {
    return `${s.committable.toLocaleString("en")} words at or under this rank would be marked known.`;
  }
  return `${s.committable.toLocaleString("en")} would be marked known · ${s.ambiguous.toLocaleString("en")} have more than one reading and are left for ordinary triage`;
}

/** Everything on this page is committable — homographs never reach it — so
 *  `total` doubles as what the commit button promises. Same shape as
 *  triage.js's `NoisePreview`: a bulk write the reader cannot see before it
 *  lands asks them to trust a threshold they have never been shown the output
 *  of. Each row also has its own one-click "not known", for the word that's
 *  common but the reader hasn't actually got — removed from the ledger's
 *  future consideration immediately, not just hidden client-side. */
function FrequencyPreview({ preview, busy, onPage, onCommit, onCancel, onExclude }) {
  if (!preview.total) {
    return html`<p class="meta-hint">Nothing left on this page — committed or excluded.</p>`;
  }
  const { total, offset, limit, terms } = preview;
  const last = Math.min(offset + terms.length, total);
  const rangeLine = `${(offset + 1).toLocaleString("en")}–${last.toLocaleString("en")} of ${total.toLocaleString("en")}, commonest first`;

  return html`
    <p class="meta-hint">${rangeLine}</p>
    <table class="days noise-table">
      <thead>
        <tr>
          <th>word</th>
          <th>reading</th>
          <th>rank</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        ${terms.map(
          (t) => html`
            <tr key=${t.term}>
              <td class="work-name">${t.term}</td>
              <td class="work-name">${t.reading || t.term}</td>
              <td>${t.rank.toLocaleString("en")}</td>
              <td>
                <button
                  class="ghost"
                  title="Mark not known — remove from this and every future list"
                  onClick=${() => onExclude(t.term, t.reading)}
                >
                  not known
                </button>
              </td>
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
        <button class="pause-btn" onClick=${onCommit} disabled=${busy}>
          ${busy ? "…" : `mark ${total.toLocaleString("en")} known`}
        </button>
        <button class="ghost" onClick=${onCancel} disabled=${busy}>
          cancel
        </button>
      </span>
    </div>
  `;
}

/** Optimistic removal of one row, keeping the preview's counters honest with
 *  what's actually still on screen. */
function removeTerm(preview, term) {
  return {
    ...preview,
    total: Math.max(0, preview.total - 1),
    terms: preview.terms.filter((t) => t.term !== term),
  };
}
