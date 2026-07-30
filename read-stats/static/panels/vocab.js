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
import { api } from "../api.js";
import { SegmentedControl } from "../components/controls.js";
import { fmtTsDate } from "../lib/format.js";
import { FrequencyView } from "./frequency.js";
import { PromotionView } from "./promotion.js";
import { TriageView } from "./triage.js";

/** The states a row is displayed in, which are not quite the states it is
 *  stored in. `status` records what the reader asserted, and `new` means only
 *  "never judged" — orthogonal to whether the word was ever met. Almost every
 *  unjudged row *has* been met (2,143 of 2,155 on 2026-07-29), so showing them
 *  all as "new" mislabels the bucket. The stored `new` is therefore split here
 *  against the encounter count, which is a fact the ingest already maintains
 *  and nothing has to assert. */
const STATES = [
  ["known", "I know this word"],
  ["seen", "met while reading, never judged"],
  ["unknown", "judged, and not known — the sweep's snooze"],
  ["new", "never met and never judged — waiting for the reading to reach it"],
  ["blacklisted", "never surface this again"],
];

const SECTIONS = [
  { value: "status", label: "status" },
  { value: "triage", label: "sweep" },
  { value: "frequency", label: "frequency" },
  { value: "promote", label: "promote" },
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
        : section === "frequency"
          ? html`<${FrequencyView}
              maxFreqRank=${settings?.triage_max_freq_rank ?? 6000}
              onCommitted=${onJudged}
            />`
          : section === "promote"
            ? html`<${PromotionView} onPromoted=${onJudged} />`
            : html`<${StatusSummary} vocab=${vocab} onImported=${onJudged} />`
    }
  `;
}

/** The counts, plus the one-shot Anki import (Pass 1: cards past Anki's
 *  new/learning queues are evidence enough to mark known outright). Split out
 *  so the segmented control above it stays put when another view replaces it. */
function StatusSummary({ vocab, onImported }) {
  const byStatus = new Map(vocab.by_status.map((s) => [s.status, s]));
  const asserted = vocab.total - (byStatus.get("new")?.total ?? 0);
  // Built whole rather than interpolated beside literal text: htm collapses
  // whitespace at a line break and prettier reflows markup there freely.
  const spellings = `(${vocab.known_in_master.toLocaleString("en")} spellings)`;
  const readyHint = `Unjudged vocabulary met at least ${vocab.ready_min_encounters} times — what a sweep can offer. Derived from the encounter count, not a stored status, so changing the threshold re-reads the whole history.`;
  // The sweep's own figure beside the standing one. `ready` cannot shrink as a
  // backlog is worked through unscoped, so it answers "how much is there" and
  // not "how much is new", which is the question the sweep is triggered by.
  const sweptOn = fmtTsDate(vocab.swept_through);
  const readySince = sweptOn
    ? `(${(vocab.ready_since ?? 0).toLocaleString("en")} since ${sweptOn})`
    : null;
  // The stored `new` bucket, split against the encounter count. Held here so
  // the table below reads from one place.
  const stateRows = new Map(byStatus);
  stateRows.set("seen", {
    total: vocab.seen,
    in_master:
      (byStatus.get("new")?.in_master ?? 0) - (vocab.never_met_vocab ?? 0),
  });
  stateRows.set("new", {
    total: vocab.never_met,
    in_master: vocab.never_met_vocab ?? 0,
  });
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState(null);
  const [err, setErr] = useState(null);

  async function importAnki() {
    setBusy(true);
    setErr(null);
    try {
      const res = await api("/api/vocab/anki-import", { method: "POST" });
      const skippedLine = res.ambiguous_skipped
        ? ` · ${res.ambiguous_skipped} skipped (more than one reading)`
        : "";
      setResult(`${res.imported} marked known${skippedLine}`);
      onImported?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  /** The jiten.moe JSON export. Sent as the file's own bytes rather than
   *  through `api()`, which re-stringifies a parsed body — pointless work on
   *  a few megabytes, and the parse would happen twice. */
  async function importJiten(event) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    setBusy(true);
    setErr(null);
    try {
      const res = await fetch("/api/vocab/jiten-import", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: await file.text(),
      });
      if (!res.ok) throw new Error((await res.text()) || res.statusText);
      const r = await res.json();
      setResult(
        `${r.terms_marked.toLocaleString("en")} spellings marked known from ` +
          `${r.resolved_entries.toLocaleString("en")} entries · ` +
          `${r.unresolved_entries.toLocaleString("en")} not in the master dictionary`,
      );
      onImported?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  return html`
    <div class="card">
      <div class="triage-actions">
        <span class="meta-hint">
          Cards past Anki's new/learning queues are marked known outright.
        </span>
        <button class="pause-btn" onClick=${importAnki} disabled=${busy}>
          ${busy ? "importing…" : "import Anki (reviewing cards)"}
        </button>
      </div>
      <div class="triage-actions">
        <span class="meta-hint">
          jiten.moe's JSON export. Keyed by JMdict entry id, so readings are
          exact and nothing is guessed. Never overwrites a word already judged.
        </span>
        <label class="pause-btn" style="cursor:pointer">
          ${busy ? "importing…" : "import jiten.moe export…"}
          <input
            type="file"
            accept=".json,application/json"
            style="display:none"
            onChange=${importJiten}
            disabled=${busy}
          />
        </label>
      </div>
      ${err && html`<p class="chart-empty">Failed: ${err}</p>`}
      ${result && html`<p class="meta-hint">${result}</p>`}
    </div>

    <div class="card">
      <div class="tile-row" style="margin-top:0">
        <div
          class="tile has-hint"
          title="Distinct words marked known, collapsing spellings of one word — 叔父 / 伯父 / おじ count once. The sub-figure is the ledger rows behind them."
        >
          <div class="label">known words</div>
          <div class="value">
            ${vocab.known_words.toLocaleString("en")}
            <span class="value-sub">${spellings}</span>
          </div>
        </div>
        <div
          class="tile has-hint"
          title="Every row in the ledger, whatever its status, dictionary word or not"
        >
          <div class="label">ledger terms</div>
          <div class="value">${vocab.total.toLocaleString("en")}</div>
        </div>
        <div class="tile has-hint" title=${readyHint}>
          <div class="label">ready to judge</div>
          <div class="value">
            ${vocab.ready.toLocaleString("en")}
            ${readySince && html`<span class="value-sub">${readySince}</span>`}
          </div>
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
          ${STATES.map(([status, hint]) => {
            const row = stateRows.get(status);
            return html`
              <tr key=${status}>
                <td>
                  <span class="vocab-swatch ${status}"></span>
                  <span title=${hint}>${status}</span>
                </td>
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
