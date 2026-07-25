// The current VN: progress through it, speed, and the controls for the
// capture window and the queue.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { ProgressBar } from "../charts.js";
import { api } from "../api.js";
import { fmtChars, fmtHours } from "../lib/format.js";
import { workProgress } from "../lib/pace.js";
import { WorkMetaForm, setCurrentWork } from "../panels/work-form.js";

export function CurrentReading({ works, settings, days, onSaved }) {
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [winBusy, setWinBusy] = useState(false);
  const [windows, setWindows] = useState([]);

  // Fetched once on mount rather than with the 60s refresh: it shells out to
  // xdotool, and the window list only changes when you launch a different VN.
  useEffect(() => {
    api("/api/vn/windows")
      .then((r) => setWindows(r.windows || []))
      .catch(() => setWindows([]));
  }, []);

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

  async function saveWindow(e) {
    e.preventDefault();
    if (!meta?.id) return;
    setWinBusy(true);
    try {
      // Per-work now, so the capture target switches with the VN. Requires the
      // work to have a metadata row, which the current one always does.
      await api(`/api/works/${meta.id}`, {
        method: "PUT",
        body: { vn_window: e.currentTarget.vnwindow.value.trim() },
      });
      onSaved();
    } catch (err) {
      alert(err.message);
    } finally {
      setWinBusy(false);
    }
  }

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Currently reading</h2>
        ${
          current &&
          html`<button class="ghost" onClick=${() => setEditing((v) => !v)}>
            ${editing ? "close" : "edit"}
          </button>`
        }
      </div>
      ${
        current &&
        prog &&
        html`
          <div class="current-work">
            ${
              meta.cover &&
              html`<img class="cover" src=${meta.cover} alt="cover" />`
            }
            <div class="info">
              <div class="title">${current.work}</div>
              <${ProgressBar}
                pct=${prog.pct}
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
                  title="Time credited to this VN, by the same presence rule the dashboard uses — gaps count up to the cap, dictionary detours included."
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
                      title="This VN's own reading speed, over all your time in it. Kept separate from other works, so switching to a harder VN shows the real drop here instead of blending into a cross-work average."
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
              <div class="title">${current.work}</div>
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
                No total length set —
                add the jpdb character count with <strong>edit</strong> to get
                progress, hours left and a finish date.
              </div>
            </div>
          </div>
        `
      }
      ${
        unmatched &&
        html`
          <div class="meta-hint">
            Nothing tracked for <strong>${title}</strong> yet. If you have been
            reading it, the title here has to match the one your tracker stamps
            on lines exactly — pick from the list below instead of typing it.
          </div>
        `
      }
      ${
        !title &&
        html`<div class="meta-hint">
          No work selected. Pick one below, or set one from the Library.
        </div>`
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
        html`
          <${WorkMetaForm}
            work=${current}
            onSaved=${onSaved}
            onCancel=${() => setEditing(false)}
          />
          <form class="now-reading" onSubmit=${saveWindow}>
            <label for="vn-window-input">VN window</label>
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
        `
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
    return "Not set for this VN — the mine button screenshots whatever has focus, which is the browser when you mine from this machine. Pick this VN's window above.";
  }
  const matches = windows.some((w) => w.includes(set));
  return matches
    ? `Screenshots this VN match "${set}". It's tied to this work, so switching VNs switches it.`
    : `No open window matches "${set}" — captures will fall back to the focused window. Re-pick it if this VN isn't running.`;
}

/** The library is where works are managed: switch the current one, edit
 *  metadata, add one you haven't started. The Currently-reading card stays
 *  read-only status so the two don't compete. */
