import { html } from 'htm/preact';
import { judged } from './state.js';
import { comprehension } from './comprehension.js';

// Three coverage figures and the work left, above the list.
//
// The tiles read live: they are computed from the same tokens the rows draw
// and the same `judged` signal, so marking a line known moves them under the
// hand that judged it. That is right here — unlike the list itself, a number
// has no place to lose.
export function ComprehensionPanel({ sentences, partial }) {
  const c = comprehension(sentences, judged.value);
  if (!c.words.total) return null;

  const left = `${c.typesLeft} word${c.typesLeft === 1 ? '' : 's'} left`;
  const leftDetail = c.typesRefused
    ? `${c.typesRefused} of them judged not known`
    : 'distinct, none judged not known yet';

  return html`
    <div class="comprehension${partial ? ' partial' : ''}">
      <${Tile}
        label="Words"
        r=${c.words}
        title="Share of content-word occurrences already known"
      />
      <${Tile}
        label="Distinct"
        r=${c.types}
        title="Share of distinct words already known — the learning left, not the reading ease"
      />
      <${Tile}
        label="Full lines"
        r=${c.lines}
        title="Share of lines with no unknown word in them"
      />
      <div class="tile" title="Distinct words in this video not marked known">
        <span class="tile-value">${left}</span>
        <span class="tile-label">${leftDetail}</span>
      </div>
    </div>
  `;
}

function Tile({ label, r, title }) {
  const value = r.exact
    ? pct(r.floor)
    : `${pct(r.floor)}–${pct(r.ceiling)}`;
  const sub = r.exact ? `${label} · all judged` : `${label} · unjudged span`;

  return html`
    <div class="tile" title=${title}>
      <span class="tile-value">${value}</span>
      <span class="tile-label">${sub}</span>
    </div>
  `;
}

function pct(x) {
  return `${Math.round(x * 100)}%`;
}
