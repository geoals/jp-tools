// The sittings that made up a day, derived ones beside manually logged ones.
//
// A table rather than a card: it is a detail of the day it belongs to, and it
// renders inside the day card's disclosure.

import { html } from "htm/preact";
import { fmtChars } from "../lib/format.js";

export function SessionsTable({ sessions }) {
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
  // A manual row's tag names what was read; with a URL it goes there. The
  // title rides on `title=` rather than in the cell — the table is a day's
  // shape, and an article headline in it would set the column width.
  const tag = (s) =>
    s.url
      ? html`<a
          class="status-tag"
          href=${s.url}
          target="_blank"
          title=${s.work || s.url}
          >${s.kind}</a
        >`
      : html`<span class="status-tag" title=${s.work || ""}>${s.kind}</span>`;
  return html`
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
              <td>${hhmm(s.start_ts)}–${hhmm(s.end_ts)} ${tag(s)}</td>
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
  `;
}
