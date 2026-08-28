// The current VN: progress through it, speed, and the controls for the
// capture window and the queue.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { ProgressBar } from "../charts.js";
import { api } from "../api.js";
import { fmtChars, fmtHours } from "../lib/format.js";
import { workProgress } from "../lib/pace.js";
import { WorkMetaForm, WorkSearchForm, setCurrentWork } from "../panels/work-form.js";
import { Modal } from "../components/modal.js";

export function CurrentReading({ works, settings, days, onSaved }) {
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [winBusy, setWinBusy] = useState(false);
  const [windows, setWindows] = useState([]);
  const [focused, setFocused] = useState(null);

  // Refetched when the dialog opens rather than only on mount: the reader's next
  // move after seeing "not set" is to click the game and come back, and a list
  // taken sixty seconds ago will not have it in.
  useEffect(() => {
    if (!editing) return;
    api("/api/vn/windows")
      .then((r) => {
        setWindows(r.windows || []);
        setFocused(r.focused || null);
      })
      .catch(() => {
        setWindows([]);
        setFocused(null);
      });
  }, [editing]);

  const title = settings.current_work;
  const current = title ? works.find((w) => w.work === title) : null;
  const meta = current?.meta;
  const prog = workProgress(current, days, settings);
  // A title that matches no work at all: usually a typo, or a work the
  // tracker hasn't stamped any lines with yet. Worth saying out loud —
  // silently rendering an empty card just looks broken.
  const unmatched = title && !current;
  // Built whole: htm collapses the whitespace where literal text meets an
  // interpolation across a line break.
  const hoursRead = current ? fmtHours(current.active_secs) : "—";
  // A plain link, not a handler: opening a work is a navigation the library
  // already owns, so back returns here and the URL is shareable.
  const detailHref = current
    ? `#library/${encodeURIComponent(current.work)}`
    : null;

  async function pick(e) {
    const next = e.currentTarget.value;
    if (!next || next === title) return;
    setBusy(true);
    try {
      await setCurrentWork(next);
      onSaved();
    } catch (err) {
      alert(err.message);
    } finally {
      setBusy(false);
    }
  }

  async function setWindow(title) {
    if (!meta?.id) return;
    setWinBusy(true);
    try {
      // Per-work now, so the capture target switches with the VN. Requires the
      // work to have a metadata row, which the current one always does.
      await api(`/api/works/${meta.id}`, {
        method: "PUT",
        body: { vn_window: title.trim() },
      });
      onSaved();
    } catch (err) {
      alert(err.message);
    } finally {
      setWinBusy(false);
    }
  }

  function saveWindow(e) {
    e.preventDefault();
    setWindow(e.currentTarget.vnwindow.value);
  }

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Currently reading</h2>
        ${
          current &&
          html`<div class="card-controls">
            <a class="ghost" href=${detailHref}>details</a>
            <button class="ghost" onClick=${() => setEditing(true)}>
              edit
            </button>
          </div>`
        }
      </div>
      ${
        current &&
        prog &&
        html`
          <div class="current-work">
            ${
              meta.cover &&
              html`<a href=${detailHref}
                ><img class="cover" src=${meta.cover} alt="cover"
              /></a>`
            }
            <div class="info">
              <div class="title">
                <a href=${detailHref}>${current.work}</a>
              </div>
              <${ProgressBar}
                pct=${prog.pct}
                done=${prog.done}
                label="Progress through ${current.work}"
              />
              <div class="progress-caption">
                <span>${prog.caption}</span>
                <span>${prog.pct.toFixed(1)}%</span>
              </div>
              <div class="tile-row">
                <div class="tile">
                  <div class="label">characters read</div>
                  <div class="value">${fmtChars(current.chars)}</div>
                </div>
                <div
                  class="tile has-hint"
                  title="Reading time for this work"
                >
                  <div class="label">hours read</div>
                  <div class="value">${hoursRead}</div>
                </div>
                ${
                  prog.started &&
                  html`
                    <div class="tile">
                      <div class="label">started</div>
                      <div class="value">${prog.started}</div>
                    </div>
                  `
                }
                ${
                  prog.done
                    ? html`
                        <div class="tile">
                          <div class="label">finished</div>
                          <div class="value">${prog.finished ?? "—"}</div>
                        </div>
                      `
                    : html`
                        ${
                          prog.remaining !== null &&
                          html`
                            <div class="tile">
                              <div class="label">remaining</div>
                              <div class="value">
                                ${fmtChars(prog.remaining)}
                              </div>
                            </div>
                          `
                        }
                        <div class="tile">
                          <div class="label">time left</div>
                          <div class="value">
                            ${
                              prog.hoursLeft !== null
                                ? `${prog.hoursLeft < 10 ? prog.hoursLeft.toFixed(1) : Math.round(prog.hoursLeft)} h`
                                : "—"
                            }
                          </div>
                        </div>
                        <div
                          class=${prog.finishHint ? "tile has-hint" : "tile"}
                          title=${
                            prog.finishHint ??
                            "No estimate: needs both a remaining count and a non-zero recent pace."
                          }
                        >
                          <div class="label">finish</div>
                          <div class="value">${prog.finish ?? "—"}</div>
                        </div>
                      `
                }
                ${
                  prog.speed !== null &&
                  html`
                    <div
                      class="tile has-hint"
                      title="Reading speed in this work"
                    >
                      <div class="label">speed</div>
                      <div class="value">
                        ${fmtChars(Math.round(prog.speed))}/h
                      </div>
                    </div>
                  `
                }
              </div>
            </div>
          </div>
        `
      }
      ${
        current &&
        !prog &&
        html`
          <div class="current-work">
            <div class="info">
              <div class="title">
                <a href=${detailHref}>${current.work}</a>
              </div>
              <div class="tile-row">
                <div class="tile">
                  <div class="label">characters read</div>
                  <div class="value">${fmtChars(current.chars)}</div>
                </div>
                <div class="tile">
                  <div class="label">hours read</div>
                  <div class="value">${hoursRead}</div>
                </div>
              </div>
              <div class="meta-hint">
                No total length set. Add it with <strong>edit</strong> for
                progress and a finish estimate.
              </div>
            </div>
          </div>
        `
      }
      ${
        unmatched &&
        html`
          <div class="meta-hint">
            Nothing tracked for <strong>${title}</strong>. The title must
            match exactly — pick from the list instead of typing.
          </div>
        `
      }
      ${
        // Asked here rather than pointed at the Library. This is the card a
        // reader is looking at when they have just started something, so it is
        // where the question belongs — and every line captured before it is
        // answered is stamped with no work at all.
        !title &&
        html`
          <div class="meta-hint">
            No work selected.
          </div>
          <${WorkSearchForm} onSaved=${onSaved} />
        `
      }
      <div class="now-reading">
        <label for="now-reading-input">Switch to</label>
        <div class="now-reading-row">
          <select
            id="now-reading-input"
            name="work"
            disabled=${busy}
            onChange=${pick}
          >
            ${
              !title &&
              html`<option value="" selected disabled>pick a work…</option>`
            }
            ${
              unmatched &&
              html`<option value=${title} selected>${title}</option>`
            }
            ${works.map(
              (w) =>
                html`<option value=${w.work} selected=${w.work === title}>
                  ${w.work}
                </option>`,
            )}
          </select>
        </div>
      </div>
      ${
        editing &&
        html`<${Modal}
          title=${`Edit ${current.work}`}
          onClose=${() => setEditing(false)}
        >
          <${WorkMetaForm}
            work=${current}
            onSaved=${onSaved}
            onCancel=${() => setEditing(false)}
          />
          <form class="now-reading" onSubmit=${saveWindow}>
            <label for="vn-window-input">VN window</label>
            ${
              // One button on a good day: the reader was looking at the game a
              // moment ago, so the window in front is almost always the answer
              // and reading thirty titles out of a list is not.
              focused &&
              focused !== meta?.vn_window &&
              html`<div class="now-reading-row">
                <button
                  type="button"
                  disabled=${winBusy}
                  onClick=${() => setWindow(focused)}
                >
                  ${`Use “${focused}”`}
                </button>
              </div>`
            }
            <div class="now-reading-row">
              <input
                id="vn-window-input"
                name="vnwindow"
                type="text"
                list="open-windows"
                value=${meta?.vn_window ?? ""}
                placeholder="pick the VN's window"
              />
              <datalist id="open-windows">
                ${windows.map((w) => html`<option value=${w}></option>`)}
              </datalist>
              <button type="submit" disabled=${winBusy}>
                ${winBusy ? "…" : "set"}
              </button>
            </div>
            <div class="meta-hint">${vnWindowHint(meta, windows)}</div>
          </form>
        <//>`
      }
    </div>
  `;
}

/** Why this box exists, and whether what's in it currently matches a real
 *  window — a stale title still mines, it just screenshots the wrong thing.
 *  The window is a property of the current work, so it travels with the VN. */

function vnWindowHint(meta, windows) {
  const set = meta?.vn_window;
  if (!set) {
    return "Not set — screenshots will capture whatever has focus.";
  }
  const matches = windows.some((w) => w.includes(set));
  return matches
    ? `Captures match "${set}".`
    : `No open window matches "${set}". Re-pick it if the VN is running.`;
}

/** The library is where works are managed: switch the current one, edit
 *  metadata, add one you haven't started. The Currently-reading card stays
 *  read-only status so the two don't compete. */
