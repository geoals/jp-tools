// The mining funnel — which lookups became cards, and which keep coming back.
//
// A body, not a card — see anki.js: it shares the Vocabulary card.

import { html } from "htm/preact";

const LOOKUP_STATUS = {
  mined: "carded",
  known: "had card",
  unmined: "not carded",
};

export function LookupsPanel({ lookups }) {
  if (!lookups || lookups.terms === 0) {
    return html`
      <p class="chart-empty">
        No lookups recorded yet. Point Yomitan's server address at /anki-proxy.
      </p>
    `;
  }
  const pct = (n) => Math.round((n / lookups.terms) * 100);
  return html`
    <div class="panel-body">
      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">words looked up</div>
          <div class="value">
            ${lookups.terms.toLocaleString("en")}
            <span class="value-sub"
              >(${lookups.events.toLocaleString("en")} lookups)</span
            >
          </div>
        </div>
        <div class="tile">
          <div class="label">became cards</div>
          <div class="value">
            ${lookups.mined.toLocaleString("en")}
            <span class="value-sub">(${pct(lookups.mined)}%)</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">already had a card</div>
          <div class="value">
            ${lookups.known.toLocaleString("en")}
            <span class="value-sub">(${pct(lookups.known)}%)</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">repeat lookups</div>
          <div class="value">
            ${lookups.repeat_events.toLocaleString("en")}
            ${
              lookups.repeat_terms > 0 &&
              html`<span class="value-sub"
                >(${lookups.repeat_terms} words)</span
              >`
            }
          </div>
        </div>
      </div>
      ${
        lookups.repeats.length > 0 &&
        html`
          <div class="word-list-label">looked up more than once</div>
          <div class="word-chips">
            ${lookups.repeats.map(
              (r) => html`
                <span class="chip"
                  >${r.term} <b>×${r.times}</b>
                  <span class="chip-note"
                    >${LOOKUP_STATUS[r.status]}</span
                  ></span
                >
              `,
            )}
            ${
              lookups.repeat_terms > lookups.repeats.length &&
              html`<span class="chip">…</span>`
            }
          </div>
        `
      }
      ${
        lookups.leeches.length > 0 &&
        html`
          <details class="never-seen">
            <summary>
              ${lookups.leech_count.toLocaleString("en")} looked up despite
              already having a card
            </summary>
            <div class="word-chips">
              ${lookups.leeches.map(
                (l) => html`
                  <span class="chip"
                    >${l.term}
                    <span class="chip-note"
                      >card ${Math.round(l.card_age_days)}d
                      old${l.times > 1 ? ` · ×${l.times}` : ""}</span
                    >
                  </span>
                `,
              )}
              ${
                lookups.leech_count > lookups.leeches.length &&
                html`<span class="chip">…</span>`
              }
            </div>
          </details>
        `
      }
      ${
        lookups.median_mine_secs !== null &&
        html`
          <div class="anki-footer">
            <span
              >median ${Math.round(lookups.median_mine_secs)}s from lookup to
              card</span
            >
          </div>
        `
      }
    </div>
  `;
}
