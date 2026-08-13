// The shelf: what is being read, and what has been finished.
//
// Cards rather than table rows, because a work is a thing with a cover and six
// numbers and a row can only carry the numbers. Clicking one opens its own
// page (`work-detail.js`) — everything per-work lives there, which is what
// keeps this level to "what is on the shelf and how far in am I".
//
// A work with no reading behind it is a cover in the queued row, not a card:
// every number on a card is zero until it has been read, and the queue is
// about which cover comes next.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { ProgressBar } from "../charts.js";
import { fmtChars, fmtDateStr, fmtHours } from "../lib/format.js";
import { workSpeedPerHour } from "../lib/pace.js";
import { WorkMetaForm } from "../panels/work-form.js";

/** Reading is anything not explicitly finished or dropped — a work with lines
 *  and no metadata row at all is being read, not filed away. */
function isFinished(w) {
  return w.meta?.status === "finished" || w.meta?.status === "dropped";
}

export function WorksShelf({ works, settings, onSaved, onOpen }) {
  const [adding, setAdding] = useState(false);

  const named = works.filter((w) => w.work);
  const read = named.filter((w) => w.chars > 0);
  const current = read.filter((w) => !isFinished(w));
  const done = read.filter(isFinished);
  // Unset queue_pos sorts last, so an ordered queue stays ordered and the rest
  // follows by title rather than by insertion order.
  const queued = named
    .filter((w) => w.chars === 0 && !isFinished(w))
    .sort(
      (a, b) =>
        (a.meta?.queue_pos ?? Infinity) - (b.meta?.queue_pos ?? Infinity) ||
        a.work.localeCompare(b.work),
    );

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
        named.length === 0
          ? html`<div class="meta-hint">
              Nothing read yet — start reading and the tracker will stamp lines
              with a title.
            </div>`
          : html`
              <div class="shelf">
                ${current.map(
                  (w) => html`
                    <${WorkCard}
                      work=${w}
                      isCurrent=${w.work === settings.current_work}
                      onOpen=${onOpen}
                    />
                  `,
                )}
              </div>
              <${CoverShelf}
                label="queued"
                works=${queued}
                caption=${queuedCaption}
                onOpen=${onOpen}
              />
              <${CoverShelf}
                label="finished"
                works=${done}
                caption=${readCaption}
                onOpen=${onOpen}
              />
            `
      }
    </div>
  `;
}

/** The whole card is one control: it opens the work. Nothing else is
 *  clickable inside it — a button within a button is ambiguous to a mouse and
 *  broken to a keyboard, so "read this next" lives on the work's own page,
 *  where there is room to say what it does. */
function WorkCard({ work: w, isCurrent, onOpen }) {
  const total = w.meta?.total_chars;
  const speed = workSpeedPerHour(w);
  // Hours left at this work's own speed. Its own, not your average: a harder
  // VN should say so in its own estimate rather than borrow an easier one's.
  const left = total && speed ? Math.max(0, total - w.chars) / speed : null;
  const pct = total ? Math.min(100, (w.chars / total) * 100) : null;
  const facts = [
    fmtChars(w.chars),
    w.active_secs > 0 ? fmtHours(w.active_secs) : null,
    speed ? `${fmtChars(Math.round(speed))}/h` : null,
  ].filter(Boolean);
  const leftLabel = left !== null ? `${fmtHours(left * 3600)} left` : null;

  return html`
    <div
      class=${isCurrent ? "work-card work-card-current" : "work-card"}
      role="button"
      tabindex="0"
      onClick=${() => onOpen(w.work)}
      onKeyDown=${(e) => e.key === "Enter" && onOpen(w.work)}
    >
      ${
        w.meta?.cover
          ? html`<img class="cover" src=${w.meta.cover} alt="" />`
          : html`<div class="cover cover-blank"></div>`
      }
      <div class="work-card-body">
        <div class="work-card-title">
          ${w.work}
          ${isCurrent && html`<span class="status-tag">current</span>`}
        </div>
        <div class="work-card-facts">${facts.join(" · ")}</div>
        ${
          pct !== null &&
          html`<div class="work-card-progress">
            <${ProgressBar} pct=${pct} />
            <div class="work-card-facts">
              ${`${pct.toFixed(0)}%${leftLabel ? ` · ${leftLabel}` : ""}`}
            </div>
          </div>`
        }
      </div>
    </div>
  `;
}

/** What was read, and the dates it was read between. */
function readCaption(w) {
  const when = [fmtDateStr(w.first_read), fmtDateStr(w.last_read)]
    .filter(Boolean)
    .join(" – ");
  return `${w.work} · ${fmtChars(w.chars)} chars${when ? ` · ${when}` : ""}`;
}

/** How long it is, which is the only number a queued work has. */
function queuedCaption(w) {
  const total = w.meta?.total_chars;
  return total ? `${w.work} · ${fmtChars(total)} chars` : w.work;
}

/** A row of covers under a label — the back catalogue, and the queue. Both are
 *  lists of works with no numbers worth a card. */
function CoverShelf({ label, works, caption, onOpen }) {
  if (!works.length) return null;
  return html`
    <div class="finished-shelf">
      <div class="word-list-label">${label}</div>
      <div class="cover-row">
        ${works.map((w) => {
          const title = caption(w);
          return html`
            <button
              type="button"
              class="cover-tile"
              title=${title}
              onClick=${() => onOpen(w.work)}
            >
              ${
                w.meta?.cover
                  ? html`<img
                      class="cover"
                      src=${w.meta.cover}
                      alt=${w.work}
                    />`
                  : html`<span class="cover cover-blank">${w.work}</span>`
              }
            </button>
          `;
        })}
      </div>
    </div>
  `;
}
