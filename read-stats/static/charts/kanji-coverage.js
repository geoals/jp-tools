// How much of everything read is covered by the N commonest kanji.
//
// Rank on x, cumulative share of all kanji encounters on y. The shape is the
// argument: it rises almost vertically over the first few hundred and then
// crawls, so the annotated 95% and 99% crossings say exactly how long the tail
// still costing you lookups is. A histogram of counts would show the same
// distribution and answer none of that.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { Tooltip, W } from "./svg.js";

/** Where the cumulative share first reaches each of these. */
const MARKS = [0.5, 0.9, 0.95, 0.99];

export function KanjiCoverageChart({ kanji, totalEncounters }) {
  const [hover, setHover] = useState(null);
  const H = 240;
  const m = { top: 16, right: 16, bottom: 28, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  if (!kanji.length || !totalEncounters) {
    return html`<p class="chart-empty">No kanji read yet.</p>`;
  }

  // kanji arrives commonest-first, so a running sum is already the curve.
  const cum = [];
  let acc = 0;
  for (const k of kanji) {
    acc += k.count;
    cum.push(acc / totalEncounters);
  }
  const n = cum.length;
  const x = (i) => m.left + (i / (n - 1 || 1)) * plotW;
  const y = (v) => m.top + plotH - v * plotH;

  const crossings = MARKS.map((share) => ({
    share,
    rank: cum.findIndex((v) => v >= share) + 1,
  })).filter((c) => c.rank > 0);

  // One point per horizontal pixel is plenty, and it keeps the path short on a
  // corpus of a few thousand kanji.
  const step = Math.max(1, Math.floor(n / plotW));
  const pts = [];
  for (let i = 0; i < n; i += step) pts.push(i);
  if (pts[pts.length - 1] !== n - 1) pts.push(n - 1);
  const line = pts
    .map((i, k) => `${k === 0 ? "M" : "L"}${x(i)},${y(cum[i])}`)
    .join(" ");
  const area = `${line} L${x(n - 1)},${y(0)} L${x(0)},${y(0)} Z`;

  const onMove = (e) => {
    const box = e.currentTarget.getBoundingClientRect();
    const frac = (e.clientX - box.left) / box.width;
    const i = Math.round(((frac * W - m.left) / plotW) * (n - 1));
    setHover(i >= 0 && i < n ? i : null);
  };

  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg
        viewBox="0 0 ${W} ${H}"
        role="img"
        aria-label="Share of all kanji read covered by the commonest N kanji"
        onMouseMove=${onMove}
      >
        ${[0, 0.25, 0.5, 0.75, 1].map(
          (t) => html`
            <line
              x1=${m.left}
              x2=${W - m.right}
              y1=${y(t)}
              y2=${y(t)}
              class="gridline"
            />
            <text x=${m.left - 6} y=${y(t) + 3} class="tick" text-anchor="end">
              ${Math.round(t * 100)}%
            </text>
          `,
        )}
        <path d=${area} fill="var(--series-1)" opacity="0.14" />
        <path d=${line} class="trend-line" stroke="var(--series-1)" />
        ${crossings.map(
          (c) => html`
            <line
              x1=${x(c.rank - 1)}
              x2=${x(c.rank - 1)}
              y1=${y(c.share)}
              y2=${y(0)}
              class="goal-line"
            />
            <text
              x=${x(c.rank - 1)}
              y=${y(c.share) - 6}
              class="tick"
              text-anchor="middle"
            >
              ${c.rank}
            </text>
          `,
        )}
        ${[0, Math.floor(n / 2), n - 1].map(
          (i) => html`
            <text x=${x(i)} y=${H - 8} class="tick" text-anchor="middle"
              >${i + 1}</text
            >
          `,
        )}
        <line
          x1=${m.left}
          x2=${W - m.right}
          y1=${y(0)}
          y2=${y(0)}
          class="baseline"
        />
        ${
          hover !== null &&
          html`<circle
            cx=${x(hover)}
            cy=${y(cum[hover])}
            r="3.5"
            class="trend-dot"
            fill="var(--series-1)"
          />`
        }
      </svg>
      <div class="chart-legend">
        ${crossings.map((c) => {
          const text = `${Math.round(c.share * 100)}% of everything read: top ${c.rank} kanji`;
          return html`<span class="legend-item legend-static">${text}</span>`;
        })}
      </div>
      ${
        hover !== null &&
        html`
          <${Tooltip} x=${x(hover)} y=${8}>
            <${CoverTip}
              rank=${hover + 1}
              row=${kanji[hover]}
              share=${cum[hover]}
            />
          <//>
        `
      }
    </div>
  `;
}

function CoverTip({ rank, row, share }) {
  const head = `#${rank} ${row.kanji}`;
  const line = `${Math.round(share * 1000) / 10}% covered`;
  const sub = `${row.count.toLocaleString("en")} encounters`;
  return html`
    <strong>${head}</strong><br />
    ${line}<br />
    <span class="tooltip-sub">${sub}</span>
  `;
}
