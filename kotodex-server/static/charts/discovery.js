// New things per day, and the running total of distinct ones.
//
// Two marks on one plot because they are the same fact at two integrations: the
// bars are how many were met for the first time that day, the line is their sum.
// The bars decay as the corpus grows, and that decay only reads as progress next
// to a line that keeps climbing.
//
// Kanji and vocabulary are the same shape, so they share the chart — each caller
// supplies its own wording and tooltip body.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { Tooltip, W, barPath, niceTicks, shortDate } from "./svg.js";

export function DiscoveryChart({ days, label, empty, barLabel, lineLabel, tip }) {
  const [hover, setHover] = useState(null);
  const H = 260;
  const m = { top: 16, right: 46, bottom: 24, left: 40 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  if (!days.length) {
    return html`<p class="chart-empty">${empty}</p>`;
  }

  const bars = niceTicks(Math.max(...days.map((d) => d.new), 1), 4);
  const yBar = (v) => m.top + plotH - (v / bars.top) * plotH;
  const totalTop = Math.max(...days.map((d) => d.cumulative), 1);
  const yLine = (v) => m.top + plotH - (v / totalTop) * plotH;

  const band = plotW / days.length;
  const barW = Math.min(22, band * 0.66);
  const labelEvery = Math.ceil(days.length / 7);
  const linePath = days
    .map(
      (d, i) =>
        `${i === 0 ? "M" : "L"}${m.left + band * i + band / 2},${yLine(d.cumulative)}`,
    )
    .join(" ");

  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg
        viewBox="0 0 ${W} ${H}"
        role="img"
        aria-label=${label}
      >
        ${bars.ticks.map(
          (t) => html`
            <line
              x1=${m.left}
              x2=${W - m.right}
              y1=${yBar(t)}
              y2=${yBar(t)}
              class="gridline"
            />
            <text
              x=${m.left - 6}
              y=${yBar(t) + 3}
              class="tick"
              text-anchor="end"
            >
              ${t}
            </text>
          `,
        )}
        ${days.map((d, i) => {
          const cx = m.left + band * i + band / 2;
          const h = yBar(0) - yBar(d.new);
          return html`
            ${
              h > 0.5 &&
              html`<path
                d=${barPath(cx - barW / 2, yBar(d.new), barW, h, 4)}
                class=${hover === i ? "bar bar-hover" : "bar"}
              />`
            }
            ${
              i % labelEvery === 0 &&
              html`<text x=${cx} y=${H - 8} class="tick" text-anchor="middle"
                >${shortDate(d.date)}</text
              >`
            }
            <rect
              x=${m.left + band * i}
              y=${m.top}
              width=${band}
              height=${plotH}
              fill="transparent"
              onMouseEnter=${() => setHover(i)}
            />
          `;
        })}
        ${/* style, not stroke=: .trend-line's CSS beats the attribute. */ ""}
        <path d=${linePath} class="trend-line" style="stroke:var(--series-2)" />
        <text
          x=${W - m.right + 4}
          y=${yLine(days[days.length - 1].cumulative) + 3}
          class="tick"
        >
          ${totalTop}
        </text>
        <line
          x1=${m.left}
          x2=${W - m.right}
          y1=${yBar(0)}
          y2=${yBar(0)}
          class="baseline"
        />
      </svg>
      <div class="chart-legend">
        <span class="legend-item legend-static">
          <span class="legend-swatch" style="background:var(--series-1)"></span
          >${barLabel}
        </span>
        <span class="legend-item legend-static">
          <span class="legend-swatch" style="background:var(--series-2)"></span
          >${lineLabel}
        </span>
      </div>
      ${
        hover !== null &&
        html`
          <${Tooltip} x=${m.left + band * hover + band / 2} y=${8}>
            <${tip} day=${days[hover]} />
          <//>
        `
      }
    </div>
  `;
}
