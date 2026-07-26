// The charts' accessible fallback: the same days as numbers.
//
// `bare` drops the card wrapper so it can sit inside another card's
// disclosure, which is where it lives on the Trends tab — it is reference for
// the charts above it, not a card of its own.

import { html } from "htm/preact";
import { fmtMins } from "../lib/format.js";

export function DaysTable({ days, todayDate, bare }) {
  const recent = days.slice(-14).reverse();
  const table = html`
    <table class="days">
      <thead>
        <tr>
          <th>date</th>
          <th>time</th>
          <th>chars</th>
          <th>chars/h</th>
          <th>lookups/1k</th>
          <th>focus</th>
        </tr>
      </thead>
      <tbody>
        ${recent.map(
          (d) => html`
            <tr class=${d.date === todayDate ? "today" : ""}>
              <td>${d.date}</td>
              <td>${d.active_secs > 0 ? fmtMins(d.active_secs) : "—"}</td>
              <td>${d.chars > 0 ? d.chars.toLocaleString("en") : "—"}</td>
              <td>
                ${
                  (d.measured?.active_secs ?? 0) >= 600
                    ? Math.round(
                        d.measured.chars / (d.measured.active_secs / 3600),
                      ).toLocaleString("en")
                    : "—"
                }
              </td>
              <td>
                ${d.lookups_per_1k !== null ? d.lookups_per_1k.toFixed(1) : "—"}
              </td>
              <td
                title=${
                  d.focus.interruptions > 0
                    ? `${d.focus.interruptions} interruptions`
                    : ""
                }
              >
                ${
                  d.focus.ratio !== null
                    ? `${Math.round(d.focus.ratio * 100)}%`
                    : "—"
                }
              </td>
            </tr>
          `,
        )}
      </tbody>
    </table>
  `;
  if (bare) return table;
  return html`
    <div class="card">
      <h2>Recent days</h2>
      ${table}
    </div>
  `;
}
