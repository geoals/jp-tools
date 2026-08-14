// A triage session over one work's script: one word at a time, judged on
// sight.
//
// The Vocab tab's sweep is a page of rows with checkboxes, and it is the right
// shape for words already met — the counts beside them carry the evidence a
// judgement is made from. Here there is no evidence to weigh: most of these
// words have never been met, so the only question is whether the reader knows
// the word, and the fastest way to ask it is one word, large, with a key under
// each answer.
//
// Three answers, not two. **Skip writes nothing** and is not a third verdict:
// it is for a term the reader should not be made to rule on — a misparse, a
// character's name, a fragment. The sweep's "unticked means unknown" rule
// cannot apply to a queue like this, where most rows are words never seen and
// leaving one alone must not be recorded as not knowing it.

import { html } from "htm/preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api.js";

/** Judgements are posted one at a time and never awaited before the next word
 *  is drawn: the session is a rhythm, and a round trip per word would set its
 *  pace. A failed write is reported and the word goes back on the queue at the
 *  next fetch, since nothing was recorded. */
export function WorkTriage({ work, onBack }) {
  const [terms, setTerms] = useState(null);
  const [at, setAt] = useState(0);
  const [tally, setTally] = useState({ known: 0, unknown: 0, skipped: 0 });
  const [error, setError] = useState(null);
  const judged = useRef(new Set());

  const load = () => {
    setError(null);
    api(`/api/works/triage?work=${encodeURIComponent(work)}`)
      .then((d) => {
        setTerms(d.terms.filter((t) => !judged.current.has(key(t))));
        setAt(0);
      })
      .catch((e) => setError(String(e.message || e)));
  };
  useEffect(load, [work]);

  const term = terms && terms[at];

  const answer = (verdict) => {
    if (!term) return;
    judged.current.add(key(term));
    setTally((t) => ({ ...t, [verdict]: t[verdict] + 1 }));
    if (verdict !== "skipped") {
      api("/api/vocab/judge", {
        method: "POST",
        body: {
          judgements: [
            {
              headword: term.headword,
              reading: term.reading,
              status: verdict,
            },
          ],
          // This is not the encounter sweep and must not move its watermark:
          // that mark says how far the reader has swept what they have *read*.
          advance_sweep: false,
        },
      }).catch((e) => setError(String(e.message || e)));
    }
    setAt((i) => i + 1);
  };

  // The keys are the session. Held on the window rather than on a focused
  // element so the reader never has to click into the page first.
  useEffect(() => {
    const onKey = (e) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const verdict = { k: "known", u: "unknown", s: "skipped" }[e.key];
      if (verdict) {
        e.preventDefault();
        answer(verdict);
      } else if (e.key === "Escape") {
        onBack();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [term, onBack]);

  const done = tally.known + tally.unknown + tally.skipped;
  const head = html`<div class="card-head">
    <h2>${work}</h2>
    <div class="card-controls">
      <button class="ghost" onClick=${onBack}>done</button>
    </div>
  </div>`;

  if (error && !terms) {
    return html`<div class="card">
      ${head}
      <p class="chart-empty">${error}</p>
    </div>`;
  }
  if (!terms) {
    return html`<div class="card">${head}
      <p class="chart-empty">Loading…</p>
    </div>`;
  }
  if (!term) {
    // Either the batch ran out or the work has nothing left to judge; the
    // fetch is the only thing that can tell the two apart.
    return html`<div class="card">
      ${head}
      <p class="chart-empty">
        Nothing left in this batch — judged ${tally.known} known,
        ${tally.unknown} unknown, skipped ${tally.skipped}.
      </p>
      <div class="triage-actions">
        <button class="ghost" onClick=${load}>next batch</button>
      </div>
    </div>`;
  }

  // Built whole: htm collapses the whitespace where a literal and an
  // interpolation straddle a line break.
  const here = `${term.count.toLocaleString("en")}× in this work`;
  const rank = term.rank ? `rank ${term.rank.toLocaleString("en")}` : "unranked";
  const met = term.met > 0 ? `met ${term.met}× while reading` : "never met";
  const progress = `${done} judged · ${terms.length - at} left in batch`;

  return html`
    <div class="card triage-card">
      ${head}
      <div class="triage-stage">
        <div class="triage-term" lang="ja">${term.headword}</div>
        ${
          term.reading &&
          term.reading !== term.headword &&
          html`<div class="triage-reading" lang="ja">${term.reading}</div>`
        }
        <div class="triage-facts">
          <span>${here}</span><span>${rank}</span><span>${met}</span>
        </div>
      </div>

      <div class="triage-actions">
        <button class="triage-btn known" onClick=${() => answer("known")}>
          known <kbd>K</kbd>
        </button>
        <button class="triage-btn unknown" onClick=${() => answer("unknown")}>
          unknown <kbd>U</kbd>
        </button>
        <button class="triage-btn skip" onClick=${() => answer("skipped")}>
          skip <kbd>S</kbd>
        </button>
      </div>

      <div class="progress-caption">
        <span>${progress}</span>
        <span>${error ?? ""}</span>
      </div>
    </div>
  `;
}

function key(t) {
  return `${t.headword}|${t.reading}`;
}
