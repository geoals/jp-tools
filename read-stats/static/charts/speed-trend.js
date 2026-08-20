// Reading speed over time, with the marks where the dominant work changed —
// so a step in the curve reads as a switch of VN rather than a regression.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { Tooltip, W, shortDate, truncWork, workChanges } from "./svg.js";

/** Candidate y-axis steps for the speed chart, finest first. */

const SPEED_STEPS = [500, 1000, 2000, 2500, 5000, 10000];

/** Chars/hour trend over days with ≥10 min read.
 *  days: [{date, active_secs, chars, work, measured, clean}]
 *
 *  `clean` — characters read across gaps holding no lookup, over what those
 *  gaps cost — draws the same measure under the other condition: the pace
 *  without the lookup tax. Only the work detail sends it; the whole-history
 *  chart draws the measured line alone. */

export function SpeedTrendChart({ days }) {
  const [hover, setHover] = useState(null);
  const H = 280;
  const m = { top: 16, right: 56, bottom: 24, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  const points = days
    .map((d, i) => ({
      ...d,
      i,
      // `measured` excludes sessions whose duration was derived from this very
      // pace — see `History::measured_days`.
      speed:
        (d.measured?.active_secs ?? 0) >= 600
          ? d.measured.chars / (d.measured.active_secs / 3600)
          : null,
    }))
    .filter((d) => d.speed !== null && d.speed > 0);

  const rawPoints = days
    .map((d, i) => ({
      ...d,
      i,
      speed:
        (d.clean?.active_secs ?? 0) >= 600 && d.clean.chars > 0
          ? d.clean.chars / (d.clean.active_secs / 3600)
          : null,
    }))
    .filter((d) => d.speed !== null);

  if (points.length < 2) {
    return html`<p class="chart-empty">
      Needs a few days with 10+ minutes read to draw a trend.
    </p>`;
  }

  const all = points.concat(rawPoints);
  const rawMax = Math.max(...all.map((p) => p.speed));
  const rawMin = Math.min(...all.map((p) => p.speed));
  // Pad the data range, then take the finest step that still keeps the axis to
  // about five gridlines. A fixed 5k step collapsed to two lines whenever the
  // spread was narrow — which is most of the time, since speed varies by
  // hundreds of chars/h day to day, not thousands.
  const pad = Math.max((rawMax - rawMin) * 0.25, 250);
  const lo = Math.max(0, rawMin - pad);
  const hi = rawMax + pad;
  const yStep = SPEED_STEPS.find((s) => (hi - lo) / s <= 5) ?? 10000;
  const yMax = Math.ceil(hi / yStep) * yStep;
  const yMin = Math.max(0, Math.floor(lo / yStep) * yStep);
  const x = (i) =>
    m.left + (days.length === 1 ? 0 : (i / (days.length - 1)) * plotW);
  const y = (v) => m.top + plotH - ((v - yMin) / (yMax - yMin)) * plotH;

  // A sub-1k step needs a decimal, or 12500 and 13000 both label as "13k".
  const kLabel = (t) => `${(t / 1000).toFixed(yStep < 1000 ? 1 : 0)}k`;
  const ticks = [];
  for (let t = yMin; t <= yMax; t += yStep) ticks.push(t);
  const path = points
    .map((p, k) => `${k === 0 ? "M" : "L"}${x(p.i)},${y(p.speed)}`)
    .join(" ");
  const rawPath = rawPoints
    .map((p, k) => `${k === 0 ? "M" : "L"}${x(p.i)},${y(p.speed)}`)
    .join(" ");
  const last = points[points.length - 1];
  const labelEvery = Math.ceil(days.length / 6);
  const changes = workChanges(days);

  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg
        viewBox="0 0 ${W} ${H}"
        role="img"
        aria-label="Reading speed trend, characters per hour"
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
              >${kLabel(t)}</text
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
          rawPoints.length > 1 &&
          html`<path
            d=${rawPath}
            class="trend-line trend-line-speed trend-line-raw"
          />`
        }
        <path d=${path} class="trend-line trend-line-speed" />
        ${
          hover !== null &&
          html`
            <line
              x1=${x(points[hover].i)}
              x2=${x(points[hover].i)}
              y1=${m.top}
              y2=${m.top + plotH}
              class="crosshair"
            />
          `
        }
        ${points.map(
          (p, k) => html`
            <circle
              cx=${x(p.i)}
              cy=${y(p.speed)}
              r=${k === points.length - 1 || hover === k ? 5 : 3.5}
              class="trend-dot trend-dot-speed"
            />
          `,
        )}
        <text x=${x(last.i) + 10} y=${y(last.speed) + 4} class="end-label">
          ${(last.speed / 1000).toFixed(1)}k/h
        </text>
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
            points.forEach((p, k) => {
              if (Math.abs(x(p.i) - px) < Math.abs(x(points[nearest].i) - px))
                nearest = k;
            });
            setHover(nearest);
          }}
        />
      </svg>
      ${
        hover !== null &&
        html`
          <${Tooltip} x=${x(points[hover].i)} y=${8}>
            <strong>${points[hover].date}</strong><br />
            ${Math.round(points[hover].speed).toLocaleString("en")} chars/h ·
            ${Math.round(points[hover].active_secs / 60)} min
            ${
              rawPoints.find((p) => p.i === points[hover].i) &&
              html`<br />${Math.round(
                rawPoints.find((p) => p.i === points[hover].i).speed,
              ).toLocaleString("en")}${" chars/h without lookups"}`
            }
          <//>
        `
      }
      ${
        rawPoints.length > 1 &&
        html`<div class="chart-legend">
          <span class="legend-item legend-static">
            <span class="legend-swatch legend-line"></span>as read
          </span>
          <span class="legend-item legend-static">
            <span class="legend-swatch legend-line legend-line-dashed"></span
            >lookups removed
          </span>
        </div>`
      }
    </div>
  `;
}
