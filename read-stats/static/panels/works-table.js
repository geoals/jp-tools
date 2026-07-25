// Every work read, with its totals — the queue and the back catalogue.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { fmtChars, fmtMins } from "../lib/format.js";
import { workSpeedPerHour } from "../lib/pace.js";
import { WorkMetaForm, setCurrentWork } from "../panels/work-form.js";

export function WorksTable({ works, settings, onSaved }) {
  // Keyed by title rather than index so the open editor follows its row when
  // the list re-sorts on refresh (it sorts by last_read).
  const [editing, setEditing] = useState(null);
  const [adding, setAdding] = useState(false);

  async function makeCurrent(title) {
    try {
      await setCurrentWork(title);
      onSaved();
    } catch (err) {
      alert(err.message);
    }
  }

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Library</h2>
        <button class="ghost" onClick=${() => setAdding((v) => !v)}>
          ${adding ? "close" : "add work"}
        </button>
      </div>
      ${
        adding &&
        html`<${WorkMetaForm}
          work=${null}
          onSaved=${() => {
            setAdding(false);
            onSaved();
          }}
          onCancel=${() => setAdding(false)}
        />`
      }
      ${
        works.length === 0
          ? html`<div class="meta-hint">
              No works yet — add one above, or just start reading and the
              tracker will stamp lines with a title.
            </div>`
          : html`<table class="days">
              <thead>
                <tr>
                  <th>title</th>
                  <th>time</th>
                  <th>chars</th>
                  <th>speed</th>
                  <th>progress</th>
                  <th>last read</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                ${works.slice(0, 10).map((w) => {
                  const isCurrent = w.work === settings.current_work;
                  return html`
                    <tr class=${isCurrent ? "row-current" : ""}>
                      <td class="work-name">
                        ${w.work ?? "(unlabeled)"}
                        ${isCurrent && html`<span class="status-tag">current</span>`}
                        ${
                          w.meta &&
                          w.meta.status !== "reading" &&
                          html`<span class="status-tag">${w.meta.status}</span>`
                        }
                      </td>
                      <td>
                        ${w.active_secs > 0 ? fmtMins(w.active_secs) : "—"}
                      </td>
                      <td>
                        ${w.chars > 0 ? w.chars.toLocaleString("en") : "—"}
                      </td>
                      <td>
                        ${
                          workSpeedPerHour(w) !== null
                            ? `${fmtChars(Math.round(workSpeedPerHour(w)))}/h`
                            : "—"
                        }
                      </td>
                      <td>
                        ${
                          w.meta?.total_chars
                            ? `${Math.min(100, (w.chars / w.meta.total_chars) * 100).toFixed(0)}%`
                            : "—"
                        }
                      </td>
                      <td>${w.last_read ?? "—"}</td>
                      <td class="row-actions">
                        ${
                          !isCurrent &&
                          w.work &&
                          html`<button
                            class="ghost"
                            onClick=${() => makeCurrent(w.work)}
                          >
                            read
                          </button>`
                        }
                        ${
                          w.meta &&
                          html`<button
                            class="ghost"
                            onClick=${() =>
                            setEditing(editing === w.work ? null : w.work)}
                          >
                            ${editing === w.work ? "close" : "edit"}
                          </button>`
                        }
                      </td>
                    </tr>
                    ${
                      editing === w.work &&
                      html`<tr>
                        <td colspan="7" class="work-editor-cell">
                          <${WorkMetaForm}
                            work=${w}
                            onSaved=${onSaved}
                            onCancel=${() => setEditing(null)}
                          />
                        </td>
                      </tr>`
                    }
                  `;
                })}
              </tbody>
            </table>`
      }
    </div>
  `;
}
