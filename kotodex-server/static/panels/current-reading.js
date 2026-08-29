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
  const [focused, setFocused] = useState(null);

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
  const vnWindow = meta?.vn_window ?? "";

  // Only while it is missing, and only for the current work: asking the desktop
  // what is open costs a round trip to the window manager, and the answer is
  // wanted for one button that is worth drawing on the day the box is empty.
  // Changing a window that is already set goes through the edit dialog, which
  // fetches the list itself.
  useEffect(() => {
    if (!current || vnWindow) return;
    api("/api/vn/windows")
      .then((r) => setFocused(r.focused || null))
      .catch(() => setFocused(null));
  }, [current?.work, vnWindow]);

  async function pick(e) {
    // The empty string is a value here: it stops reading anything, which is
    // what a reader who has put a VN down and not started another one means.
    const next = e.currentTarget.value;
    if (next === title) return;
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

  // Through the current work rather than by id: the title may have no metadata
  // row yet — the tracker stamps lines with whatever is current — and the
  // endpoint creates one. The edit dialog writes the same column by id, because
  // there it is editing a work that need not be the one being read.
  async function setWindow(name) {
    setWinBusy(true);
    try {
      await api("/api/vn/window", { method: "PUT", body: { window: name } });
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
              <dl class="tile-row">
                <div class="tile">
                  <dt class="label">characters read</dt>
                  <dd class="value">${fmtChars(current.chars)}</dd>
                </div>
                <div
                  class="tile has-hint"
                  title="Reading time for this work"
                >
                  <dt class="label">hours read</dt>
                  <dd class="value">${hoursRead}</dd>
                </div>
                ${
                  prog.started &&
                  html`
                    <div class="tile">
                      <dt class="label">started</dt>
                      <dd class="value">${prog.started}</dd>
                    </div>
                  `
                }
                ${
                  prog.done
                    ? html`
                        <div class="tile">
                          <dt class="label">finished</dt>
                          <dd class="value">${prog.finished ?? "—"}</dd>
                        </div>
                      `
                    : html`
                        ${
                          prog.remaining !== null &&
                          html`
                            <div class="tile">
                              <dt class="label">remaining</dt>
                              <dd class="value">
                                ${fmtChars(prog.remaining)}
                              </dd>
                            </div>
                          `
                        }
                        <div class="tile">
                          <dt class="label">time left</dt>
                          <dd class="value">
                            ${
                              prog.hoursLeft !== null
                                ? `${prog.hoursLeft < 10 ? prog.hoursLeft.toFixed(1) : Math.round(prog.hoursLeft)} h`
                                : "—"
                            }
                          </dd>
                        </div>
                        <div
                          class=${prog.finishHint ? "tile has-hint" : "tile"}
                          title=${
                            prog.finishHint ??
                            "No estimate: needs both a remaining count and a non-zero recent pace."
                          }
                        >
                          <dt class="label">finish</dt>
                          <dd class="value">${prog.finish ?? "—"}</dd>
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
                      <dt class="label">speed</dt>
                      <dd class="value">
                        ${fmtChars(Math.round(prog.speed))}/h
                      </dd>
                    </div>
                  `
                }
              </dl>
            </div>
          </div>
        `
      }
      ${
        current &&
        !prog &&
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
              <dl class="tile-row">
                <div class="tile">
                  <dt class="label">characters read</dt>
                  <dd class="value">${fmtChars(current.chars)}</dd>
                </div>
                <div class="tile">
                  <dt class="label">hours read</dt>
                  <dd class="value">${hoursRead}</dd>
                </div>
              </dl>
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
            Nothing being read. Lines captured now are stamped with no title,
            so they count towards the day but towards no work.
          </div>
          <${WorkSearchForm} settings=${settings} onSaved=${onSaved} />
        `
      }
      ${
        // On the card rather than inside the edit dialog, because "where do I
        // tell it which window the game is" was the question the dialog was
        // the answer to and nothing said so. Not set is the state worth
        // drawing: the overlay cannot follow the game until it is.
        //
        // A book has no window: nothing hooks it, and the overlay is not over
        // anything.
        current &&
        current.kind !== "book" &&
        html`
          <div class="now-reading">
            <label>Game window</label>
            <div class="now-reading-row">
              ${
                vnWindow
                  ? html`<span class="work-window-name">${vnWindow}</span>`
                  : html`<span class="meta-hint">
                      not set — the overlay cannot follow the game
                    </span>`
              }
              ${
                !vnWindow &&
                focused &&
                html`<button
                  class="ghost"
                  disabled=${winBusy}
                  onClick=${() => setWindow(focused)}
                >
                  ${winBusy ? "…" : `use “${focused}”`}
                </button>`
              }
              <button class="ghost" onClick=${() => setEditing(true)}>
                ${vnWindow ? "change" : "choose…"}
              </button>
            </div>
          </div>
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
            <option value="" selected=${!title}>— nothing —</option>
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
            isCurrent=${true}
            onSaved=${onSaved}
            onCancel=${() => setEditing(false)}
            onDeleted=${() => {
              setEditing(false);
              onSaved();
            }}
          />
        <//>`
      }
    </div>
  `;
}

/** The library is where works are managed: switch the current one, edit
 *  metadata, add one you haven't started. The Currently-reading card stays
 *  read-only status so the two don't compete. */
