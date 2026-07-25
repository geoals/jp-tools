// The mined deck: how much of it the reading has shown you again.
//
// A body, not a card — it shares the Vocabulary card with the lookup funnel,
// because both answer "what happened to the words", not "how much time".

import { html } from "htm/preact";

export function AnkiPanel({ anki, onRefresh, busy }) {
  if (!anki) return null;
  if (!anki.available) {
    return html`
      <div class="meta-hint">
        No deck snapshot yet — open Anki (desktop or phone) and refresh.
        ${" "}
        <button class="pause-btn" onClick=${onRefresh} disabled=${busy}>
          ${busy ? "refreshing…" : "↻ refresh from Anki"}
        </button>
      </div>
    `;
  }
  const pct =
    anki.mined > 0 ? ((anki.reencountered / anki.mined) * 100).toFixed(0) : 0;
  const ageMins = Math.round((Date.now() / 1000 - anki.snapshot_ts) / 60);
  // One string, not markup: line breaks between text and ${...} inside an
  // element get collapsed by htm, so a prettier reflow eats the spaces.
  const snapshotAge = `snapshot ${
    ageMins < 60 ? `${ageMins} min` : `${Math.round(ageMins / 60)} h`
  } ago`;
  return html`
    <div class="panel-body">
      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">mined words</div>
          <div class="value">${anki.mined.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">re-encountered</div>
          <div class="value">
            ${anki.reencountered.toLocaleString("en")}
            <span class="value-sub">(${pct}%)</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">encounters · 7d</div>
          <div class="value">${anki.week_encounters.toLocaleString("en")}</div>
        </div>
      </div>
      ${
        anki.top_week.length > 0 &&
        html`
          <div class="word-list-label">most met this week</div>
          <div class="word-chips">
            ${anki.top_week.map(
              (w) =>
                html`<span class="chip">${w.word} <b>×${w.count}</b></span>`,
            )}
          </div>
        `
      }
      ${
        anki.never_count > 0 &&
        html`
          <details class="never-seen">
            <summary>
              ${anki.never_count.toLocaleString("en")} mined words not
              re-encountered yet
            </summary>
            <div class="word-chips">
              ${anki.never_sample.map(
                (w) => html`<span class="chip">${w}</span>`,
              )}
              ${
                anki.never_count > anki.never_sample.length &&
                html`<span class="chip">…</span>`
              }
            </div>
          </details>
        `
      }
      <div class="anki-footer">
        <span>${snapshotAge}</span>
        <button class="pause-btn" onClick=${onRefresh} disabled=${busy}>
          ${busy ? "refreshing…" : "↻ refresh"}
        </button>
      </div>
    </div>
  `;
}
