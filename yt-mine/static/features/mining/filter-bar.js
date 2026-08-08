import { html } from 'htm/preact';
import { judged } from './state.js';
import { FILTERS, matchesFilter } from './ledger.js';

// The row above the list. One group now; it is a row rather than a control so
// sorting and the next filter have somewhere to go.
export function FilterBar({ sentences, filter, onFilter }) {
  const marks = judged.value;
  const counts = Object.fromEntries(
    FILTERS.map(([id]) => [id, sentences.filter((s) => matchesFilter(s, id, marks)).length]),
  );

  return html`
    <div class="toolbar">
      <div class="segmented" role="group">
        ${FILTERS.map(([id, label]) => html`
          <button
            class=${filter === id ? 'on' : ''}
            onClick=${() => onFilter(id)}
            title=${TITLES[id]}
          >
            <span>${label}</span><span class="count">${counts[id]}</span>
          </button>
        `)}
      </div>
    </div>
  `;
}

const TITLES = {
  all: 'Every line',
  unknown: 'Lines holding a word not marked known',
  'i+1': 'Lines holding exactly one word not marked known',
};
