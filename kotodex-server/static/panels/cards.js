// What the reading knows about the mined deck that Anki does not.
//
// Anki schedules on its own review history. The line stream is a second record
// of the same words — a word met without a lookup was recalled, a word looked
// up on a long interval was not — and this is that evidence, sorted into the
// four readings a card can support.
//
// **It reports and writes nothing.** The thresholds are a guess until the
// buckets have been read against words already judged known, and one encounter
// day can be a skimmed line — acting on that in bulk is the
// wrong-assertion-at-scale case the ledger's two-signal rule exists to prevent.
//
// Fetched on demand rather than with the dashboard poll: it asks Anki for the
// whole deck's scheduling state and its review log, which is seconds of work
// and fails outright when Anki is shut.
//
// Each row shows the ledger key beside the card's own spelling. A surprising
// verdict is nearly always a surprising key — the card says ラクダ, the ledger
// says 駱駝, and the reading that only ever wrote it in katakana looks like a
// word never met.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";

/** The buckets, in the order they are worth reading. `bring_forward` first:
 *  it is the one signal Anki structurally cannot have. */
const BUCKETS = [
  {
    id: "bring_forward",
    label: "Looked up anyway",
    hint: "Mature cards you looked up. Anki thinks these are known; the reading says otherwise.",
    columns: ["interval", "lookups", "met"],
  },
  {
    id: "defer",
    label: "Met, never looked up",
    hint: "Met on several days since the last review, never looked up.",
    columns: ["interval", "met", "since"],
  },
  {
    id: "retire",
    label: "Done",
    hint: "Long interval, met repeatedly, never looked up.",
    columns: ["interval", "met", "since"],
  },
  {
    id: "never_met",
    label: "Never met since mined",
    hint: "Mined but never met again in the reading.",
    columns: ["interval", "age"],
  },
];

export function CardsView() {
  const [data, setData] = useState(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);
  const [open, setOpen] = useState("bring_forward");

  async function load() {
    setBusy(true);
    setErr(null);
    try {
      setData(await api("/api/anki/cards"));
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  if (err) {
    return html`
      <div class="card">
        <p class="chart-empty">Failed: ${err}</p>
        <button class="pause-btn" onClick=${load} disabled=${busy}>retry</button>
      </div>
    `;
  }

  if (!data) {
    return html`
      <div class="card">
        <p class="chart-empty">
          Every mined card against what the reading has since shown you.
        </p>
        <button class="pause-btn" onClick=${load} disabled=${busy}>
          ${busy ? "asking Anki…" : "build the report"}
        </button>
      </div>
    `;
  }

  if (!data.available) {
    return html`
      <div class="card">
        <p class="chart-empty">
          No AnkiConnect reachable — the deck's scheduling state only exists in
          Anki.
        </p>
        <button class="pause-btn" onClick=${load} disabled=${busy}>retry</button>
      </div>
    `;
  }

  const t = data.thresholds;
  const scope = `${data.cards.toLocaleString("en")} cards, ${data.reviewing.toLocaleString("en")} in review`;

  return html`
    <div class="card">
      <h2>Cards vs. reading</h2>
      <p class="meta-hint" title="Read-only. Nothing here changes a card.">
        ${scope}
      </p>
      ${
        (data.unmirrored > 0 || data.unlogged > 0) &&
        html`<${Caveats} unmirrored=${data.unmirrored} unlogged=${data.unlogged} />`
      }
      <div class="triage-actions">
        <span class="meta-hint" title="Every threshold this report applies.">
          ${thresholdLine(t)}
        </span>
        <button class="pause-btn" onClick=${load} disabled=${busy}>
          ${busy ? "asking Anki…" : "↻ rebuild"}
        </button>
      </div>
    </div>
    ${BUCKETS.map((b) => {
      const rows = data.buckets?.[b.id] ?? [];
      const total = data.counts?.[b.id] ?? 0;
      return html`<${Bucket}
        key=${b.id}
        spec=${b}
        rows=${rows}
        total=${total}
        listed=${data.listed_per_bucket}
        open=${open === b.id}
        onToggle=${() => setOpen(open === b.id ? null : b.id)}
      />`;
    })}
  `;
}

/** The two ways a card can be missing from the join, both worth seeing rather
 *  than papering over: no ledger key, or no review log. */
function Caveats({ unmirrored, unlogged }) {
  const lines = [];
  if (unmirrored > 0) {
    lines.push(
      `${unmirrored.toLocaleString("en")} cards are not in the deck snapshot, so they have no ledger key to join on — refresh from Anki`,
    );
  }
  if (unlogged > 0) {
    lines.push(
      `${unlogged.toLocaleString("en")} cards have no review log; their window starts at the card's last modification instead`,
    );
  }
  return html`
    <ul class="meta-hint">
      ${lines.map((l) => html`<li>${l}</li>`)}
    </ul>
  `;
}

function thresholdLine(t) {
  return [
    `looked up: interval ≥ ${t.mature_days}d`,
    `met: ${t.defer_days} days`,
    `done: ${t.retire_days} days and interval ≥ ${t.retire_interval}d`,
    `never met: card older than ${t.never_met_age_days}d`,
  ].join(" · ");
}

function Bucket({ spec, rows, total, listed, open, onToggle }) {
  const heading = `${spec.label} — ${total.toLocaleString("en")}`;
  const truncated =
    total > rows.length
      ? `showing the first ${listed.toLocaleString("en")}`
      : null;

  return html`
    <details
      class="card card-bucket"
      open=${open}
      onToggle=${(e) => e.currentTarget.open !== open && onToggle()}
    >
      <summary><h2>${heading}</h2></summary>
      <p class="meta-hint">${spec.hint}</p>
      ${
        rows.length === 0
          ? html`<p class="chart-empty">Nothing in this bucket.</p>`
          : html`
              ${truncated && html`<p class="meta-hint">${truncated}</p>`}
              <table class="days">
                <thead>
                  <tr>
                    <th>card</th>
                    <th>ledger key</th>
                    ${spec.columns.map((c) => html`<th key=${c}>${COLUMN[c].label}</th>`)}
                  </tr>
                </thead>
                <tbody>
                  ${rows.map(
                    (r) => html`
                      <tr key=${r.note_id}>
                        <td>${r.vocab}</td>
                        <td class=${r.key === r.vocab ? "meta-hint" : ""}>
                          ${r.key === r.vocab ? "" : r.key}
                        </td>
                        ${spec.columns.map(
                          (c) => html`<td key=${c}>${COLUMN[c].value(r)}</td>`,
                        )}
                      </tr>
                    `,
                  )}
                </tbody>
              </table>
            `
      }
    </details>
  `;
}

/** Each column built whole in JS — htm collapses a line break between text and
 *  an interpolation, which renders `4 days` as `4days`. */
const COLUMN = {
  interval: { label: "interval", value: (r) => `${r.interval} d` },
  lookups: { label: "looked up", value: (r) => `${r.lookups}×` },
  met: {
    label: "met since review",
    value: (r) => `${days(r.encounter_days)} · ${r.encounters}×`,
  },
  since: { label: "last review", value: (r) => `${r.since_review_days} d ago` },
  age: {
    label: "met ever",
    value: (r) => days(r.encounter_days_all),
  },
};

const days = (n) => `${n} ${n === 1 ? "day" : "days"}`;
