// The history, over one range: how much, how fast, and what it cost.
//
// Four cards became one because they were four slices of the same 30 days, each
// with its own hardcoded window — "Last 7 days" could not be asked about 30,
// and the charts could not be asked about 7. One range selector drives all of
// them now, and the week tiles are what they always were: a caption for the
// bars, not a card.
//
// The speed panel and the rate panel are stacked rather than merged. Chars/hour
// runs in the thousands and events/hour in the tens, so one plot would need two
// y-scales and where those line up is a choice, not a fact — the same reason
// the day timeline keeps two panels. Stacked, they share an x position: a
// speed dip and a lookup spike on the same day sit in the same vertical slice,
// which is the whole lookup-tax argument in one glance.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { DailyBarChart, RateTrendChart, SpeedTrendChart } from "../charts.js";
import { SegmentedControl } from "../components/controls.js";
import { DaysTable } from "./days-table.js";
import { fmtChars, fmtMins } from "../lib/format.js";

/** Ranges offered, in days. Capped at what `/api/days` loads. */

const RANGES = [
  { value: "7", label: "7d" },
  { value: "30", label: "30d" },
  { value: "60", label: "60d" },
];

/** Below this much reading in the range an average speed is noise. */

const MIN_RATE_SECS = 600;

export function TrendsCard({ days, dialogue, targetMins, todayDate }) {
  const [range, setRange] = useState("30");
  const [metric, setMetric] = useState("minutes");
  const [split, setSplit] = useState(false);

  const n = Number(range);
  const win = days.slice(-n);

  const chars = win.reduce((a, d) => a + d.chars, 0);
  const secs = win.reduce((a, d) => a + d.active_secs, 0);
  const daysRead = win.filter((d) => d.active_secs > 0).length;
  const avgSpeed =
    secs >= MIN_RATE_SECS
      ? `${fmtChars(Math.round(chars / (secs / 3600)))}/h`
      : "—";
  const daysReadLabel = `${daysRead}/${win.length}`;

  const byDate = {};
  for (const d of dialogue?.days ?? []) byDate[d.date] = d;

  // Minutes and characters get one chart with a switch rather than two charts,
  // because they are the same question — how much reading happened — asked in
  // the two units it has. They are deliberately never overlaid: that would need
  // a second y-scale, and where two scales line up is a choice rather than a
  // fact. Reading speed is exactly the relationship between them, and it has
  // its own panel below.
  const barsTitle = metric === "minutes" ? "Daily minutes" : "Daily characters";

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Trends</h2>
        <${SegmentedControl}
          label="Range"
          value=${range}
          onChange=${setRange}
          options=${RANGES}
        />
      </div>

      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">time</div>
          <div class="value">${fmtMins(secs)}</div>
        </div>
        <div class="tile">
          <div class="label">characters</div>
          <div class="value">${chars.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">days read</div>
          <div class="value">${daysReadLabel}</div>
        </div>
        <div class="tile">
          <div class="label">avg speed</div>
          <div class="value">${avgSpeed}</div>
        </div>
      </div>

      <div class="day-chart-head">
        <h3>${barsTitle}</h3>
        <div class="card-controls">
          <${SegmentedControl}
            label="Measure"
            value=${metric}
            onChange=${setMetric}
            options=${[
              { value: "minutes", label: "minutes" },
              { value: "chars", label: "chars" },
            ]}
          />
          <button
            type="button"
            class=${split ? "toggle-btn toggle-on" : "toggle-btn"}
            aria-pressed=${split}
            onClick=${() => setSplit(!split)}
          >
            ⬓ split by dialogue
          </button>
        </div>
      </div>
      <${DailyBarChart}
        days=${win}
        dialogueByDate=${byDate}
        metric=${metric}
        split=${split}
        targetMins=${targetMins}
      />

      <div class="day-chart-head">
        <h3>Reading speed · chars/hour</h3>
      </div>
      <${SpeedTrendChart} days=${win} />

      <div class="day-chart-head">
        <h3>Lookups & cards · per hour read</h3>
      </div>
      <${RateTrendChart} days=${win} />

      <details class="day-sessions">
        <summary>day by day</summary>
        <${DaysTable} days=${win} todayDate=${todayDate} bare=${true} />
      </details>
    </div>
  `;
}
