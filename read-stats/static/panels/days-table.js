// The charts' accessible fallback: the same days as numbers.

import { html } from "htm/preact";
import { fmtMins } from "../lib/format.js";

export function DaysTable({ days, todayDate }) {
  const recent = days.slice(-14).reverse();
  return html`
    <div class="card">
      <h2>Recent days</h2>
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
                    d.active_secs >= 600
                      ? Math.round(
                          d.chars / (d.active_secs / 3600),
                        ).toLocaleString("en")
                      : "—"
                  }
                </td>
                <td>
                  ${
                    d.lookups_per_1k !== null
                      ? d.lookups_per_1k.toFixed(1)
                      : "—"
                  }
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
    </div>
  `;
}
