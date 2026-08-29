import { html } from "htm/preact";

/** `done` paints the bar green, track included: a finished work is not "in
 *  progress, further along", which is what a full blue bar reads as. It is the
 *  same green every other met threshold on the dashboard uses. */

export function ProgressBar({ pct, label, done }) {
  return html`
    <div
      class=${done ? "meter done" : "meter"}
      role="meter"
      aria-valuenow=${Math.round(pct)}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-label=${label}
    >
      <div
        class=${done ? "meter-fill done" : "meter-fill"}
        style="width:${Math.min(100, pct)}%"
      ></div>
    </div>
  `;
}

/** One target, no intermediate mark: a second threshold on the same bar asks the
 *  reader to hold two goals at once, and the lower one becomes the real one. */

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
