// What is being read, and what the reading turned into.
//
// The tab for everything that isn't a number about time: the works themselves,
// the words they produced, and the one *action* on the dashboard — logging a
// session by hand. That action used to be a permanent card in a column of
// statistics, which is what it isn't: it is behind a button here, and the
// button lives next to the works it adds a session to.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { AnkiPanel } from "./anki.js";
import { DialogueCard } from "./dialogue.js";
import { LogForm } from "./log-form.js";
import { LookupsPanel } from "./lookups.js";
import { SegmentedControl } from "../components/controls.js";
import { WorksTable } from "./works-table.js";

const VOCAB_VIEWS = [
  { value: "lookups", label: "what lookups became" },
  { value: "anki", label: "re-encounters" },
];

export function LibraryView({
  works,
  settings,
  anki,
  lookups,
  dialogue,
  onRefreshAnki,
  ankiBusy,
  onSaved,
}) {
  const [logging, setLogging] = useState(false);
  const [vocab, setVocab] = useState("lookups");

  return html`
    <${WorksTable} works=${works} settings=${settings} onSaved=${onSaved} />

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

    <${DialogueCard} dialogue=${dialogue} currentWork=${settings.current_work} />

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
