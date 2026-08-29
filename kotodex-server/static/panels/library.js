// What is being read, and the one action on the dashboard — logging a session by
// hand, behind a button beside the works it adds a session to.
//
// Two levels. The shelf lists the works; opening one replaces the whole tab
// with that work's page rather than expanding a row in place — everything
// per-work goes there, and there is more of it than a row can hold.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { LogForm } from "./log-form.js";
import { Modal } from "../components/modal.js";
import { WorkDetail } from "./work-detail.js";
import { WorksShelf } from "./works-shelf.js";

export function LibraryView({ works, settings, openWork, onSaved }) {
  const [logging, setLogging] = useState(false);

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
        <h2>Log a session</h2>
        <button class="ghost" onClick=${() => setLogging(true)}>+ log</button>
      </div>
      <div
        class="meta-hint"
        title="A physical book, or a VN read before Kotodex"
      >
        For reading that wasn't hooked.
      </div>
      ${
        logging &&
        html`<${Modal}
          title="Log a session"
          onClose=${() => setLogging(false)}
        >
          <${LogForm}
            onLogged=${() => {
              setLogging(false);
              onSaved();
            }}
          />
        <//>`
      }
    </div>
  `;
}
