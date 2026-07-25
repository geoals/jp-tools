// One day, whole: the goal, the day's totals, its curve, and its sittings.
//
// These were three cards — Today, Day detail, Today's sessions — asking the
// same question at the same scale, with the middle one carrying a date picker
// the other two didn't answer to. They are one card with one date now, so
// stepping back a day moves the totals and the sessions with the chart.
//
// The totals come from the already-loaded `days` array rather than a fetch:
// `/api/days` returns exactly what the hero needs for every day in the window,
// so the date nav costs nothing until it reaches the timeline.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { DayTimelineChart, GoalMeter } from "../charts.js";
import { SessionsTable } from "./sessions.js";
import { api } from "../api.js";
import { fmtChars, fmtMins } from "../lib/format.js";

/** Smoothing windows offered by the granularity slider, in minutes. */

const SMOOTH_STEPS = [1, 2, 3, 5, 8, 12, 20, 30, 45];

/** Below this much reading a per-hour rate is noise, not a measurement. */

const MIN_RATE_SECS = 600;

export function DayCard({ days, todayDate, goal }) {
  const [date, setDate] = useState(todayDate);
  const [smoothIdx, setSmoothIdx] = useState(3); // 5 min
  const [timeline, setTimeline] = useState(null);
  const [sessions, setSessions] = useState(null);
  const [err, setErr] = useState(null);

  // The timeline and the sessions are the only per-date fetches; both are
  // cheap and both are skipped entirely while the card sits on today, which is
  // where it sits almost always.
  useEffect(() => {
    let live = true;
    setTimeline(null);
    setSessions(null);
    setErr(null);
    api(`/api/day/timeline?date=${date}`)
      .then((d) => live && setTimeline(d))
      .catch((e) => live && setErr(e.message));
    api(`/api/sessions?date=${date}`)
      .then((d) => live && setSessions(d))
      .catch(() => live && setSessions(null));
    return () => {
      live = false;
    };
  }, [date]);

  const shift = (n) => {
    const d = new Date(`${date}T12:00:00`);
    d.setDate(d.getDate() + n);
    setDate(d.toISOString().slice(0, 10));
  };

  const day = days.find((d) => d.date === date) ?? {
    date,
    chars: 0,
    active_secs: 0,
    lookups: 0,
    cards: 0,
    focus: { ratio: null, longest_stretch_secs: 0 },
  };
  const mins = day.active_secs / 60;
  const rated = day.active_secs >= MIN_RATE_SECS;
  const speed = rated ? day.chars / (day.active_secs / 3600) : null;
  const lookupsPerHour = rated ? day.lookups / (day.active_secs / 3600) : null;

  // Sub-values are built as strings so prettier can't reflow the markup and
  // change the rendered spacing.
  const lookupRate =
    lookupsPerHour !== null ? `(${lookupsPerHour.toFixed(1)}/h)` : null;
  const bestStretch =
    day.focus.longest_stretch_secs > 0
      ? `(${fmtMins(day.focus.longest_stretch_secs)} best)`
      : null;
  const targetLabel = `${goal.target_mins} min`;
  const isToday = date === todayDate;
  const windowMins = SMOOTH_STEPS[smoothIdx];
  const sessionCount =
    (sessions?.derived?.length ?? 0) + (sessions?.manual?.length ?? 0);
  const sessionSummary = `${sessionCount} session${sessionCount === 1 ? "" : "s"} · ${fmtMins(day.active_secs)}`;
  // On a past day "min to target" is a verdict, not a nudge — say what
  // happened rather than what is left to do.
  const goalNote =
    mins >= goal.target_mins
      ? html`<span class="goal-met">target met</span>`
      : isToday
        ? `${Math.ceil(goal.target_mins - mins)} min to target`
        : `${Math.round(mins)} of ${goal.target_mins} min`;

  return html`
    <div class="card">
      <div class="card-head">
        <h2>${isToday ? "Today" : "Day"}</h2>
        <div class="day-controls">
          <button class="day-nav" onClick=${() => shift(-1)} title="Previous day">
            ◀
          </button>
          <input
            type="date"
            value=${date}
            max=${todayDate}
            onInput=${(e) => e.target.value && setDate(e.target.value)}
          />
          <button
            class="day-nav"
            onClick=${() => shift(1)}
            disabled=${isToday}
            title="Next day"
          >
            ▶
          </button>
          ${
            !isToday &&
            html`<button
              class="day-nav day-nav-wide"
              onClick=${() => setDate(todayDate)}
            >
              today
            </button>`
          }
        </div>
      </div>

      <div class="hero-row">
        <span class="hero">${fmtMins(day.active_secs)}</span>
        <span class="hero-sub">${goalNote}</span>
      </div>
      <${GoalMeter} mins=${mins} targetMins=${goal.target_mins} />
      <div class="meter-caption">
        <span>0</span><span>${targetLabel}</span>
      </div>

      <div class="tile-row">
        <div class="tile">
          <div class="label">characters</div>
          <div class="value">${day.chars.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">speed</div>
          <div class="value">
            ${speed ? `${fmtChars(Math.round(speed))}/h` : "—"}
          </div>
        </div>
        <div class="tile">
          <div class="label">cards mined</div>
          <div class="value">${day.cards > 0 ? day.cards : "—"}</div>
        </div>
        <div class="tile">
          <div class="label">lookups</div>
          <div class="value">
            ${day.lookups > 0 ? day.lookups.toLocaleString("en") : "—"}
            ${lookupRate && html`<span class="value-sub">${lookupRate}</span>`}
          </div>
        </div>
        <div class="tile">
          <div class="label">focus</div>
          <div class="value">
            ${
              day.focus.ratio !== null
                ? `${Math.round(day.focus.ratio * 100)}%`
                : "—"
            }
            ${bestStretch && html`<span class="value-sub">${bestStretch}</span>`}
          </div>
        </div>
      </div>

      <div class="day-chart-head">
        <h3>Reading speed vs. lookups</h3>
        <label class="smooth-control">
          smoothing
          <input
            type="range"
            min="0"
            max=${SMOOTH_STEPS.length - 1}
            step="1"
            value=${smoothIdx}
            onInput=${(e) => setSmoothIdx(Number(e.target.value))}
          />
          <span class="smooth-value">${`${windowMins} min`}</span>
        </label>
      </div>
      ${err && html`<p class="chart-empty">Failed to load: ${err}</p>`}
      ${!err && !timeline && html`<p class="chart-empty">Loading…</p>`}
      ${
        timeline &&
        html`<${DayTimelineChart}
          buckets=${timeline.buckets}
          bucketSecs=${timeline.bucket_secs}
          windowMins=${windowMins}
        />`
      }
      ${
        sessionCount > 0 &&
        html`
          <details class="day-sessions">
            <summary>${sessionSummary}</summary>
            <${SessionsTable} sessions=${sessions} />
          </details>
        `
      }
    </div>
  `;
}
