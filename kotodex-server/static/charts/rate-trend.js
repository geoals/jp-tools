// The unknown-word rate over time: lookups per 1000 characters, which is what
// says whether a work sits at the comprehension edge.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import {
  Tooltip,
  W,
  niceCeil,
  rateStep,
  shortDate,
  truncWork,
  workChanges,
} from "./svg.js";

// Both series are events per hour, so they share one y-axis. Minutes read is a
// different unit and stays in its own chart — overlaying it here would mean two
// y-scales, whose alignment is arbitrary and invents correlations.

const RATE_SERIES = [
  {
    key: "lookups",
    label: "lookups/h",
    color: "var(--series-1)",
    of: (d) => d.lookups,
  },
  {
    key: "cards",
    label: "cards/h",
    color: "var(--series-2)",
    of: (d) => d.cards,
  },
];

/** Minimum active time before a per-hour rate means anything. */

const RATE_MIN_SECS = 600;

/**
 * Lookups and mined cards per hour of reading, toggleable.
 * days: [{date, active_secs, lookups, cards}]
 */

export function RateTrendChart({ days }) {
  const [hover, setHover] = useState(null);
  const [off, setOff] = useState({});
  const H = 280;
  const m = { top: 16, right: 64, bottom: 24, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  // Rate is per *hour of reading*, so a day is only a data point once it has
  // enough active time for the denominator to be stable.
  const rated = days
    // Measured hours, like every other rate — see `History::measured_days`.
    .map((d, i) => ({ ...d, i, hours: (d.measured?.active_secs ?? 0) / 3600 }))
    .filter((d) => (d.measured?.active_secs ?? 0) >= RATE_MIN_SECS);

  const shown = RATE_SERIES.filter((s) => !off[s.key]);

  if (rated.length < 2) {
    return html`<p class="chart-empty">
      Needs a few days with 10+ minutes read to draw a trend.
    </p>`;
  }

  const values = shown.flatMap((s) => rated.map((d) => s.of(d) / d.hours));
  const step = rateStep(Math.max(...values, 1));
  const yMax = niceCeil(Math.max(...values, 1) * 1.1, step);
  const x = (i) =>
    m.left + (days.length === 1 ? 0 : (i / (days.length - 1)) * plotW);
  const y = (v) => m.top + plotH - (v / yMax) * plotH;

  const ticks = [];
  for (let t = 0; t <= yMax; t += step) ticks.push(t);
  const labelEvery = Math.ceil(days.length / 6);
  const changes = workChanges(days);

  const plotted = shown.map((s) => {
    const pts = rated.map((d) => ({ i: d.i, v: s.of(d) / d.hours }));
    return {
      s,
      pts,
      last: pts[pts.length - 1],
      path: pts
        .map((p, k) => `${k === 0 ? "M" : "L"}${x(p.i)},${y(p.v)}`)
        .join(" "),
    };
  });

  // End labels sit at their line's last value, but two series finishing close
  // together would overprint — push them apart, keeping the higher one on top.
  const labelled = plotted
    .map((p) => ({ ...p, labelY: y(p.last.v) + 4 }))
    .sort((a, b) => a.labelY - b.labelY);
  for (let k = 1; k < labelled.length; k += 1) {
    const gap = labelled[k].labelY - labelled[k - 1].labelY;
    if (gap < 13) labelled[k].labelY = labelled[k - 1].labelY + 13;
  }

  // Identity is legend + direct label, never color alone.
  const legend = html`
    <div class="chart-legend">
      ${RATE_SERIES.map(
        (s) => html`
          <button
            type="button"
            class=${`legend-item${off[s.key] ? " legend-off" : ""}`}
            aria-pressed=${!off[s.key]}
            onClick=${() => setOff((o) => ({ ...o, [s.key]: !o[s.key] }))}
          >
            <span class="legend-swatch" style=${`background:${s.color}`}></span
            >${s.label}
          </button>
        `,
      )}
    </div>
  `;

  if (shown.length === 0) {
    return html`${legend}
      <p class="chart-empty">Both series hidden — pick one above.</p>`;
  }

  return html`
    ${legend}
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg
        viewBox="0 0 ${W} ${H}"
        role="img"
        aria-label="Lookups and mined cards per hour of reading, last ${days.length} days"
      >
        ${ticks.map(
          (t) => html`
            <line
              x1=${m.left}
              x2=${W - m.right}
              y1=${y(t)}
              y2=${y(t)}
              class="gridline"
            />
            <text x=${m.left - 6} y=${y(t) + 3} class="tick" text-anchor="end"
              >${t}</text
            >
          `,
        )}
        ${days.map(
          (d, i) =>
            i % labelEvery === 0 &&
            html`
              <text x=${x(i)} y=${H - 8} class="tick" text-anchor="middle"
                >${shortDate(d.date)}</text
              >
            `,
        )}
        ${changes.map(
          (c) => html`
            <line
              x1=${x(c.i)}
              x2=${x(c.i)}
              y1=${m.top}
              y2=${m.top + plotH}
              class="work-divider"
            />
            <text
              x=${x(c.i) + 4}
              y=${m.top + 10}
              class="work-divider-label"
              transform=${`rotate(90 ${x(c.i) + 4} ${m.top + 10})`}
            >
              ${truncWork(c.work)}
            </text>
          `,
        )}
        ${
          hover !== null &&
          html`
            <line
              x1=${x(rated[hover].i)}
              x2=${x(rated[hover].i)}
              y1=${m.top}
              y2=${m.top + plotH}
              class="crosshair"
            />
          `
        }
        ${plotted.map(
          ({ s, pts, path }) => html`
            <path d=${path} class="trend-line" style=${`stroke:${s.color}`} />
            ${pts.map(
              (p, k) => html`
                <circle
                  cx=${x(p.i)}
                  cy=${y(p.v)}
                  r=${k === pts.length - 1 || hover === k ? 5 : 3.5}
                  class="trend-dot"
                  style=${`fill:${s.color}`}
                />
              `,
            )}
          `,
        )}
        ${labelled.map(
          ({ s, last, labelY }) => html`
            <text x=${x(last.i) + 8} y=${labelY} class="end-label"
              >${last.v.toFixed(1)}</text
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
        <rect
          x=${m.left}
          y=${m.top}
          width=${plotW}
          height=${plotH}
          fill="transparent"
          onMouseMove=${(e) => {
            const rect = e.currentTarget.closest("svg").getBoundingClientRect();
            const px = ((e.clientX - rect.left) / rect.width) * W;
            let nearest = 0;
            rated.forEach((d, k) => {
              if (Math.abs(x(d.i) - px) < Math.abs(x(rated[nearest].i) - px))
                nearest = k;
            });
            setHover(nearest);
          }}
        />
      </svg>
      ${
        hover !== null &&
        html`
          <${Tooltip} x=${x(rated[hover].i)} y=${8}>
            <strong>${rated[hover].date}</strong><br />
            ${shown.map(
              (s) => html`
                ${s.label.replace("/h", "")}:
                ${(s.of(rated[hover]) / rated[hover].hours).toFixed(1)}/h<br />
              `,
            )}
            <span class="tooltip-sub"
              >${Math.round(rated[hover].measured.active_secs / 60)} min
              read</span
            >
          <//>
        `
      }
    </div>
  `;
}
