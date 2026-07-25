// The two non-chart indicators: a progress bar, and the goal meter.

import { html } from "htm/preact";

/** Plain progress bar (same visual language as the goal meter, no marker). */

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

/** Goal meter: fill in the series hue, unfilled track a lighter step of the
 *  same ramp. One target, no intermediate mark — a second threshold on the same
 *  bar asked the reader to hold two goals at once and made the first one the
 *  real one. */

export function GoalMeter({ mins, targetMins }) {
  const pct = Math.min(100, (mins / targetMins) * 100);
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
    </div>
  `;
}
