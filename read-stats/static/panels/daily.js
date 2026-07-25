// The daily bar chart and the week-at-a-glance tiles under it.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { DailyBarChart } from "../charts.js";
import { SegmentedControl } from "../components/controls.js";
import { fmtChars, fmtMins } from "../lib/format.js";

export function DailyChartCard({ days, dialogue, targetMins }) {
  const [metric, setMetric] = useState("minutes");
  const [split, setSplit] = useState(false);

  const byDate = {};
  for (const d of dialogue?.days ?? []) byDate[d.date] = d;

  const heading = metric === "minutes" ? "Daily minutes" : "Daily characters";
  const title = `${heading} · last 30 days`;
  return html`
    <div class="card">
      <div class="card-head">
        <h2>${title}</h2>
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
        days=${days.slice(-30)}
        dialogueByDate=${byDate}
        metric=${metric}
        split=${split}
        targetMins=${targetMins}
      />
    </div>
  `;
}

export function WeekTiles({ days }) {
  const week = days.slice(-7);
  const chars = week.reduce((a, d) => a + d.chars, 0);
  const secs = week.reduce((a, d) => a + d.active_secs, 0);
  const daysMet = week.filter((d) => d.active_secs > 0).length;
  return html`
    <div class="card">
      <h2>Last 7 days</h2>
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
          <div class="value">${daysMet}/7</div>
        </div>
        <div class="tile">
          <div class="label">avg speed</div>
          <div class="value">
            ${
              secs >= 600
                ? `${fmtChars(Math.round(chars / (secs / 3600)))}/h`
                : "—"
            }
          </div>
        </div>
      </div>
    </div>
  `;
}

/** This work's own reading speed in chars/hour, or null below a 10-minute
 *  floor where the denominator is noise. This is the number that halves when
 *  you switch to a harder VN — kept per-work so nothing averages it against
 *  another title. */
