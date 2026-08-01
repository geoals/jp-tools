// The daily bar chart: minutes or characters per day.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import {
  Tooltip,
  W,
  barPath,
  kChars,
  niceCeil,
  niceTicks,
  shortDate,
} from "./svg.js";

const BAR_COLOR = "var(--series-1)";

/** Daily reading, as minutes or as characters. days: [{date, active_secs, chars}] */
export function DailyBarChart({ days, metric, targetMins }) {
  const [hover, setHover] = useState(null);
  const H = 300;
  // Right margin holds the "goal 120" label — wide enough that
  // three-digit goals don't run off the viewBox.
  const m = { top: 16, right: 56, bottom: 24, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;
  const isMins = metric === "minutes";

  const total = (d) => (isMins ? d.active_secs / 60 : d.chars);

  const maxV = Math.max(...days.map(total), 0);
  let ticks;
  let yMax;
  if (isMins) {
    const yStep = targetMins >= 120 ? 60 : 30;
    yMax = niceCeil(Math.max(maxV, targetMins * 1.15), yStep);
    ticks = [];
    for (let t = 0; t <= yMax; t += yStep) ticks.push(t);
  } else {
    const nice = niceTicks(Math.max(maxV, 1), 5);
    ticks = nice.ticks;
    yMax = nice.top;
  }
  const y = (v) => m.top + plotH - (v / yMax) * plotH;

  const band = plotW / days.length;
  const barW = Math.min(24, band * 0.7);
  const labelEvery = Math.ceil(days.length / 7);

  if (maxV <= 0) {
    return html`<p class="chart-empty">No reading recorded yet.</p>`;
  }

  const label = isMins ? "minutes" : "characters";
  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg
        viewBox="0 0 ${W} ${H}"
        role="img"
        aria-label="Daily reading ${label}, last ${days.length} days"
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
            <text x=${m.left - 6} y=${y(t) + 3} class="tick" text-anchor="end">
              ${isMins ? t : kChars(t)}
            </text>
          `,
        )}
        ${
          isMins &&
          [[targetMins, "goal"]].map(
            ([v, name]) => html`
              <line
                x1=${m.left}
                x2=${W - m.right}
                y1=${y(v)}
                y2=${y(v)}
                class="goal-line"
              />
              <text x=${W - m.right + 4} y=${y(v) + 3} class="tick"
                >${name} ${v}</text
              >
            `,
          )
        }
        ${days.map((d, i) => {
          const cx = m.left + band * i + band / 2;
          const v = total(d);
          const h = y(0) - y(v);
          const dim = hover === null || hover === i ? 1 : 0.55;
          return html`
            ${
              h > 0.5 &&
              html`
                <path
                  d=${barPath(cx - barW / 2, y(v), barW, h, 4)}
                  fill=${BAR_COLOR}
                  opacity=${dim}
                />
              `
            }
            ${
              i % labelEvery === 0 &&
              html`
                <text x=${cx} y=${H - 8} class="tick" text-anchor="middle"
                  >${shortDate(d.date)}</text
                >
              `
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
        <line
          x1=${m.left}
          x2=${W - m.right}
          y1=${y(0)}
          y2=${y(0)}
          class="baseline"
        />
      </svg>
      ${
        hover !== null &&
        html`
          <${Tooltip} x=${m.left + band * hover + band / 2} y=${8}>
            <${DayBarTooltip} day=${days[hover]} />
          <//>
        `
      }
    </div>
  `;
}

function DayBarTooltip({ day }) {
  const headline = `${Math.round(day.active_secs / 60)} min · ${day.chars.toLocaleString("en")} chars`;
  return html`<strong>${day.date}</strong><br />${headline}`;
}
