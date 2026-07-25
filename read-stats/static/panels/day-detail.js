// One day's intra-day curve, with the smoothing control over it.
//
// The server returns deliberately fine buckets (a minute by default) and
// smoothing happens here, so dragging the control never refetches and can't
// perturb a session still in progress.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { DayTimelineChart } from "../charts.js";
import { api } from "../api.js";

const SMOOTH_STEPS = [1, 2, 3, 5, 8, 12, 20, 30, 45];

/**
 * One day, minute by minute. Owns the date and the smoothing window.
 *
 * The buckets arrive raw (one minute) and are smoothed in the browser, so
 * dragging the granularity slider is instant and issues no request — which
 * also keeps it off the server while a reading session is live.
 */

export function DayDetailCard({ todayDate }) {
  const [date, setDate] = useState(todayDate);
  const [smoothIdx, setSmoothIdx] = useState(3); // 5 min
  const [data, setData] = useState(null);
  const [err, setErr] = useState(null);

  useEffect(() => {
    let live = true;
    setData(null);
    setErr(null);
    api(`/api/day/timeline?date=${date}`)
      .then((d) => live && setData(d))
      .catch((e) => live && setErr(e.message));
    return () => {
      live = false;
    };
  }, [date]);

  const shift = (days) => {
    const d = new Date(`${date}T12:00:00`);
    d.setDate(d.getDate() + days);
    setDate(d.toISOString().slice(0, 10));
  };

  const windowMins = SMOOTH_STEPS[smoothIdx];
  const sessions = data?.sessions ?? [];
  const totalMins = sessions.reduce((a, s) => a + s.active_secs, 0) / 60;

  return html`
    <div class="card">
      <h2>Day detail · reading speed vs. lookups</h2>
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
          disabled=${date >= todayDate}
          title="Next day"
        >
          ▶
        </button>
        ${
          date !== todayDate &&
          html`<button
            class="day-nav"
            style="width:auto;padding:0 8px"
            onClick=${() => setDate(todayDate)}
          >
            today
          </button>`
        }
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
      ${!err && !data && html`<p class="chart-empty">Loading…</p>`}
      ${
        data &&
        html`
          <${DayTimelineChart}
            buckets=${data.buckets}
            bucketSecs=${data.bucket_secs}
            windowMins=${windowMins}
          />
          ${
            sessions.length > 0 &&
            html`
              <div class="meta-hint">
                ${`${sessions.length} session${sessions.length === 1 ? "" : "s"} · ${Math.round(totalMins)} min read`}
              </div>
            `
          }
        `
      }
    </div>
  `;
}
