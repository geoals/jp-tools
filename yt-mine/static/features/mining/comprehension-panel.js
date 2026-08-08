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

  return html`
    <div class="comprehension${partial ? ' partial' : ''}">
      <${Tile}
        label="Words"
        s=${c.words}
        title="Content-word occurrences marked known, of all of them — how much of the talking is covered"
      />
      <${Tile}
        label="Distinct"
        s=${c.types}
        title="Distinct words marked known, of all of them — the learning, not the reading ease"
      />
      <${Tile}
        label="Full lines"
        s=${c.lines}
        title="Lines with every content word marked known"
      />
      <${Tile} label="Left" s=${null} left=${c.left} />
    </div>
  `;
}

function Tile({ label, s, left, title }) {
  const value = s ? `${Math.round(s.pct * 100)}%` : String(left);
  const sub = s ? `${label} · ${s.known} of ${s.total}` : 'Distinct, not known';

  return html`
    <div class="tile" title=${title ?? 'Distinct words in this video not marked known'}>
      <span class="tile-value">${value}</span>
      <span class="tile-label">${sub}</span>
    </div>
  `;
}
