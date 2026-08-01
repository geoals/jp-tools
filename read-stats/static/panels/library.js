// What is being read, and what the reading turned into.
//
// The tab for everything that isn't a number about time: the works themselves,
// the words they produced, and the one *action* on the dashboard — logging a
// session by hand. That action used to be a permanent card in a column of
// statistics, which is what it isn't: it is behind a button here, and the
// button lives next to the works it adds a session to.
//
// Two levels. The shelf lists the works; opening one replaces the whole tab
// with that work's page rather than expanding a row in place — everything
// per-work goes there, and there is more of it than a row can hold. The
// vocabulary and log-form cards belong to the shelf level: they are about the
// reading as a whole, not about any one work.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { AnkiPanel } from "./anki.js";
import { LogForm } from "./log-form.js";
import { LookupsPanel } from "./lookups.js";
import { SegmentedControl } from "../components/controls.js";
import { WorkDetail } from "./work-detail.js";
import { WorksShelf } from "./works-shelf.js";

const VOCAB_VIEWS = [
  { value: "lookups", label: "what lookups became" },
  { value: "anki", label: "re-encounters" },
];

export function LibraryView({
  works,
  settings,
  anki,
  lookups,
  openWork,
  onRefreshAnki,
  ankiBusy,
  onSaved,
}) {
  const [logging, setLogging] = useState(false);
  const [vocab, setVocab] = useState("lookups");

  // Which work is open lives in the URL, not in state: opening one is a
  // navigation, so the browser's back button returns to the shelf instead of
  // leaving the tab, and a link to a work survives a reload.
  const openAt = (title) => {
    location.hash = `#library/${encodeURIComponent(title)}`;
  };

  if (openWork) {
    return html`<${WorkDetail}
      work=${openWork}
      works=${works}
      settings=${settings}
      onBack=${() => {
        location.hash = "#library";
      }}
      onSaved=${onSaved}
    />`;
  }

  return html`
    <${WorksShelf}
      works=${works}
      settings=${settings}
      onSaved=${onSaved}
      onOpen=${openAt}
    />

    <div class="card">
      <div class="card-head">
        <h2>Vocabulary</h2>
        <div class="card-controls">
          <${SegmentedControl}
            label="View"
            value=${vocab}
            onChange=${setVocab}
            options=${VOCAB_VIEWS}
          />
          ${
            vocab === "anki" &&
            html`<button
              class="ghost"
              onClick=${onRefreshAnki}
              disabled=${ankiBusy}
            >
              ${ankiBusy ? "refreshing…" : "↻ refresh"}
            </button>`
          }
        </div>
      </div>
      ${
        vocab === "anki"
          ? html`<${AnkiPanel}
              anki=${anki}
              onRefresh=${onRefreshAnki}
              busy=${ankiBusy}
            />`
          : html`<${LookupsPanel} lookups=${lookups} />`
      }
    </div>

    <div class="card">
      <div class="card-head">
        <h2>Log a session</h2>
        <button class="ghost" onClick=${() => setLogging((v) => !v)}>
          ${logging ? "close" : "+ log"}
        </button>
      </div>
      ${
        logging
          ? html`<${LogForm}
              onLogged=${() => {
                setLogging(false);
                onSaved();
              }}
            />`
          : html`<div class="meta-hint">
              For reading the line stream never saw — a physical book, or a VN
              read before this existed. Everything else is tracked already.
            </div>`
      }
    </div>
  `;
}
