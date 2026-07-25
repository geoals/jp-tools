// The two non-chart indicators: a progress bar, and the goal meter with its
// floor and target marks.

import { html } from "htm/preact";

export function ProgressBar({ pct, label }) {
  return html`
    <div
      class="meter"
      role="meter"
      aria-valuenow=${Math.round(pct)}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-label=${label}
    >
      <div class="meter-fill" style="width:${Math.min(100, pct)}%"></div>
    </div>
  `;
}

/** Goal meter: fill in the series hue, unfilled track a lighter step of the same ramp. */

export function GoalMeter({ mins, floorMins, targetMins }) {
  const pct = Math.min(100, (mins / targetMins) * 100);
  const floorPct = Math.min(100, (floorMins / targetMins) * 100);
  return html`
    <div
      class="meter"
      role="meter"
      aria-valuenow=${Math.round(mins)}
      aria-valuemin="0"
      aria-valuemax=${targetMins}
      aria-label="Minutes read toward ${targetMins}-minute target"
    >
      <div class="meter-fill" style="width:${pct}%"></div>
      <div
        class="meter-marker"
        style="left:${floorPct}%"
        title="floor ${floorMins} min"
      ></div>
    </div>
  `;
}
