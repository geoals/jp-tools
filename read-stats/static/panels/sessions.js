// The sittings that made up today, derived ones beside manually logged ones.

import { html } from "htm/preact";
import { fmtChars } from "../lib/format.js";

export function SessionsToday({ sessions }) {
  if (!sessions) return null;
  const rows = [
    ...sessions.derived.map((s) => ({ ...s, kind: "vn" })),
    ...sessions.manual.map((s) => ({
      ...s,
      kind: s.source,
      active_secs: s.end_ts - s.start_ts,
    })),
  ].sort((a, b) => a.start_ts - b.start_ts);
  if (!rows.length) return null;
  const hhmm = (ts) => new Date(ts * 1000).toTimeString().slice(0, 5);
  return html`
    <div class="card">
      <h2>Today's sessions</h2>
      <table class="days">
        <thead>
          <tr>
            <th>time</th>
            <th>mins</th>
            <th>chars</th>
            <th>chars/h</th>
            <th>cards</th>
            <th>cards/h</th>
          </tr>
        </thead>
        <tbody>
          ${rows.map((s) => {
            const hours = s.active_secs / 3600;
            return html`
              <tr>
                <td>
                  ${hhmm(s.start_ts)}–${hhmm(s.end_ts)}
                  <span class="status-tag">${s.kind}</span>
                </td>
                <td>${Math.round(s.active_secs / 60)}</td>
                <td>${s.chars.toLocaleString("en")}</td>
                <td>
                  ${
                    s.active_secs >= 600
                      ? fmtChars(Math.round(s.chars / hours))
                      : "—"
                  }
                </td>
                <td>${s.cards > 0 ? s.cards : "—"}</td>
                <td>
                  ${
                    s.cards > 0 && s.active_secs >= 600
                      ? (s.cards / hours).toFixed(1)
                      : "—"
                  }
                </td>
              </tr>
            `;
          })}
        </tbody>
      </table>
    </div>
  `;
}

/** Smoothing windows offered by the granularity slider, in minutes. */
