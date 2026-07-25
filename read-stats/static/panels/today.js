// Today against the goal: the meter, the streak, and the day's totals.

import { html } from "htm/preact";
import { GoalMeter } from "../charts.js";
import { fmtChars, fmtMins } from "../lib/format.js";

export function TodayCard({ summary }) {
  const { today, goal } = summary;
  const mins = today.active_secs / 60;
  const speed =
    today.active_secs >= 600 ? today.chars / (today.active_secs / 3600) : null;
  // Same 10-minute floor as speed: below it the per-hour denominator is noise.
  const lookupsPerHour =
    today.active_secs >= 600
      ? today.lookups / (today.active_secs / 3600)
      : null;
  // Sub-values are built as strings so prettier can't reflow the markup and
  // change the rendered spacing.
  const lookupRate =
    lookupsPerHour !== null ? `(${lookupsPerHour.toFixed(1)}/h)` : null;
  const bestStretch =
    today.focus.longest_stretch_secs > 0
      ? `(${fmtMins(today.focus.longest_stretch_secs)} best)`
      : null;
  return html`
    <div class="card">
      <h2>Today · ${today.date}</h2>
      <div class="hero-row">
        <span class="hero">${fmtMins(today.active_secs)}</span>
        <span class="hero-sub">
          ${
            mins >= goal.target_mins
              ? html`<span class="goal-met">target met</span>`
              : mins >= goal.floor_mins
                ? html`<span class="goal-met">floor met</span> ·
                    ${Math.ceil(goal.target_mins - mins)} min to target`
                : `${Math.ceil(goal.floor_mins - mins)} min to floor`
          }
        </span>
      </div>
      <${GoalMeter}
        mins=${mins}
        floorMins=${goal.floor_mins}
        targetMins=${goal.target_mins}
      />
      <div class="meter-caption">
        <span>0</span><span>floor ${goal.floor_mins}</span
        ><span>${goal.target_mins} min</span>
      </div>
      <div class="tile-row">
        <div class="tile">
          <div class="label">characters</div>
          <div class="value">${today.chars.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">speed</div>
          <div class="value">
            ${speed ? `${fmtChars(Math.round(speed))}/h` : "—"}
          </div>
        </div>
        <div class="tile">
          <div class="label">cards mined</div>
          <div class="value">${today.cards > 0 ? today.cards : "—"}</div>
        </div>
        <div class="tile">
          <div class="label">lookups</div>
          <div class="value">
            ${today.lookups > 0 ? today.lookups.toLocaleString("en") : "—"}
            ${lookupRate && html`<span class="value-sub">${lookupRate}</span>`}
          </div>
        </div>
        <div class="tile">
          <div class="label">focus</div>
          <div class="value">
            ${
              today.focus.ratio !== null
                ? `${Math.round(today.focus.ratio * 100)}%`
                : "—"
            }
            ${
              bestStretch && html`<span class="value-sub">${bestStretch}</span>`
            }
          </div>
        </div>
      </div>
    </div>
  `;
}

/** A row of mutually exclusive choices, styled as one segmented control. */
