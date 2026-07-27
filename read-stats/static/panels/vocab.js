// The vocabulary tab: what the knowledge ledger holds, by status.
//
// Two numbers are being shown, and they are not the same question. `total` is
// every ledger row — every term the reading has ever produced, dictionary word
// or not. `in_master` is the subset the master dictionary lists, which is the
// only one that means anything as a vocabulary count: Jitendex gives headwords
// to phrases (ああでもないこうでもない), so counting rows would inflate the
// figure without bound. See spec/knowledge-db.md.
//
// The statuses are listed in full even at zero. A status missing from the
// response means no row carries it, and that is worth seeing — while nothing
// has been triaged, every row is `new` and the five assertion statuses are all
// zero, which is the honest picture of where the ledger stands.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { SegmentedControl } from "../components/controls.js";
import { TriageView } from "./triage.js";

/** Every status, in the order the triage passes fill them, with what each one
 *  means. `new` first because it is the default and, today, the whole table. */
const STATUSES = [
  ["new", "ingested from reading, never judged"],
  ["known", "I know this word"],
  ["learning", "actively being learned"],
  ["unknown", "judged, and not known"],
  ["name", "a proper noun, not vocabulary"],
  ["blacklisted", "never surface this again"],
];

const SECTIONS = [
  { value: "status", label: "status" },
  { value: "triage", label: "triage" },
];

export function VocabView({ vocab, settings, onJudged }) {
  const [section, setSection] = useState("status");

  if (!vocab) return html`<p class="chart-empty">Loading…</p>`;

  if (!vocab.total) {
    return html`
      <div class="card">
        <h2>vocabulary</h2>
        <p class="chart-empty">
          The ledger is empty — no reading has been tokenized into it yet.
          <code>POST /api/vocab/rebuild</code> fills it from the whole history.
        </p>
      </div>
    `;
  }

  return html`
    <div class="card">
      <h2>vocabulary</h2>
      <${SegmentedControl}
        value=${section}
        options=${SECTIONS}
        onChange=${setSection}
        label="vocabulary view"
      />
    </div>
    ${
      section === "triage"
        ? html`<${TriageView}
            minEncounters=${settings?.triage_min_encounters ?? 3}
            onJudged=${onJudged}
          />`
        : html`<${StatusSummary} vocab=${vocab} />`
    }
  `;
}

/** The counts. Split out so the segmented control above it stays put when the
 *  triage view replaces it. */
function StatusSummary({ vocab }) {
  const byStatus = new Map(vocab.by_status.map((s) => [s.status, s]));
  const asserted = vocab.total - (byStatus.get("new")?.total ?? 0);

  return html`
    <div class="card">
      <div class="tile-row" style="margin-top:0">
        <div
          class="tile has-hint"
          title="Terms marked known that the master dictionary lists — the vocabulary scale"
        >
          <div class="label">known words</div>
          <div class="value">
            ${vocab.known_in_master.toLocaleString("en")}
          </div>
        </div>
        <div
          class="tile has-hint"
          title="Every row in the ledger, whatever its status, dictionary word or not"
        >
          <div class="label">ledger terms</div>
          <div class="value">${vocab.total.toLocaleString("en")}</div>
        </div>
        <div
          class="tile has-hint"
          title="Rows carrying any status other than new — i.e. something I have actually judged"
        >
          <div class="label">triaged</div>
          <div class="value">
            ${asserted.toLocaleString("en")}
            <span class="value-sub"
              >(${((asserted / vocab.total) * 100).toFixed(0)}%)</span
            >
          </div>
        </div>
      </div>
    </div>

    <div class="card">
      <h2>by status</h2>
      <table class="days">
        <thead>
          <tr>
            <th>status</th>
            <th>terms</th>
            <th>vocabulary</th>
          </tr>
        </thead>
        <tbody>
          ${STATUSES.map(([status, hint]) => {
            const row = byStatus.get(status);
            return html`
              <tr key=${status}>
                <td><span title=${hint}>${status}</span></td>
                <td>${(row?.total ?? 0).toLocaleString("en")}</td>
                <td>${(row?.in_master ?? 0).toLocaleString("en")}</td>
              </tr>
            `;
          })}
        </tbody>
      </table>
      <div class="meta-hint">
        <strong>vocabulary</strong> counts only terms the master dictionary
        lists — the rest are phrases, names and reading noise, which belong in
        the ledger but not in a vocabulary figure.
      </div>
    </div>
  `;
}
