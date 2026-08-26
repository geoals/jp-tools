// How a ledger row was actually written, and a line to show for it.
//
// The ledger keys on Sudachi's normalized form, so a row saying 窺う may only
// ever have appeared as うかがう, and 何 may have been ナニ. Two pages ask the
// same question of it — triage, deciding whether it knows a word, and the
// tokenize page, deciding whether the pipeline found one — so the fetch and the
// table live here rather than twice.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";

/** The ledger's key as a string. `(headword, reading)`, never headword alone:
 *  空/そら and 空/から are different words. */
export function termKey(t) {
  return `${t.headword}\t${t.reading || ""}`;
}

/** Open/close state for a set of rows, plus the fetch.
 *
 *  Only the rows actually opened are fetched — a queue is 200 rows and a pasted
 *  paragraph is dozens, and nobody expands them all. */
export function useSpellings() {
  const [open, setOpen] = useState({});

  async function toggle(t) {
    const k = termKey(t);
    if (k in open) {
      setOpen((s) => {
        const next = { ...s };
        delete next[k];
        return next;
      });
      return;
    }
    setOpen((s) => ({ ...s, [k]: null }));
    try {
      const q = new URLSearchParams({
        headword: t.headword,
        reading: t.reading || "",
      });
      const res = await api(`/api/vocab/surfaces?${q}`);
      setOpen((s) => ({ ...s, [k]: res }));
    } catch (e) {
      setOpen((s) => ({ ...s, [k]: { error: e.message } }));
    }
  }

  return {
    toggle,
    isOpen: (t) => termKey(t) in open,
    data: (t) => open[termKey(t)],
  };
}

/** One term's spellings, most common first, each with a line it appeared in.
 *
 *  Counts are per surface with inflection kept, so 出来る and 出来れ are separate
 *  rows. Folding them would need the same normalization that lost the
 *  orthography in the first place. */
export function Spellings({ data }) {
  if (data === undefined || data === null) {
    return html`<p class="meta-hint">Loading…</p>`;
  }
  if (data.error) return html`<p class="meta-hint">Failed: ${data.error}</p>`;
  if (!data.surfaces.length) {
    return html`<p class="meta-hint">
      Nothing recorded — this word has not been met in hooked reading since the
      surface sink existed, or was only met in logged session text, which has no
      line ids.
    </p>`;
  }
  return html`
    <table class="days spellings-table">
      <tbody>
        ${data.surfaces.map((s) => {
          const seen = `×${s.count.toLocaleString("en")}`;
          return html`
            <tr key=${s.surface}>
              <td class="jp triage-word">${s.surface}</td>
              <td class="num">${seen}</td>
              <td class="jp">${s.line ?? "—"}</td>
            </tr>
          `;
        })}
      </tbody>
    </table>
  `;
}
