// The daily bar chart: minutes or characters per day, stackable by source and
// by the dialogue/narration split.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { Tooltip, W, barPath, kChars, niceCeil, niceTicks, segments, shortDate } from "./svg.js";

const DAY_SPLIT = [
  { key: "dialogue", label: "dialogue", color: "var(--series-1)" },
  { key: "narration", label: "narration", color: "var(--series-2)" },
  { key: "other", label: "no line text", color: "var(--muted)" },
];

/**
 * Daily reading, as minutes or as characters, optionally stacked by whether
 * the text was dialogue.
 *
 * `dialogueByDate` supplies the split; a date missing from it (or a day whose
 * parts fall short of the total, which is what a manual session looks like)
 * lands in the remainder segment. The parts are never rescaled to fill the
 * bar — the bar's height stays the day's real total in both modes, so toggling
 * the split changes what the bar is made of and never how tall it is.
 *
 * days: [{date, active_secs, chars}]
 */

export function DailyBarChart({
  days,
  dialogueByDate,
  metric,
  split,
  floorMins,
  targetMins,
}) {
  const [hover, setHover] = useState(null);
  const H = 300;
  // Right margin holds the "goal 120" / "floor 60" labels — wide enough that
  // three-digit goals don't run off the viewBox.
  const m = { top: 16, right: 56, bottom: 24, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;
  const isMins = metric === "minutes";

  const total = (d) => (isMins ? d.active_secs / 60 : d.chars);

  /** A day's bar as stacked parts, largest category first, bottom-up. */
  const parts = (d) => {
    const t = total(d);
    if (!split) return [{ ...DAY_SPLIT[0], key: "all", value: t }];
    const s = dialogueByDate[d.date];
    const dv = s ? (isMins ? s.dialogue_secs / 60 : s.dialogue_chars) : 0;
    const nv = s ? (isMins ? s.narration_secs / 60 : s.narration_chars) : 0;
    // Clamped: the parts are derived on a slightly different day boundary from
    // the total (a gap straddling the rollover), so they can overshoot by a
    // few seconds. Never let that draw a bar taller than the day.
    const other = Math.max(0, t - dv - nv);
    return [dv, nv, other].map((value, k) => ({ ...DAY_SPLIT[k], value }));
  };

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
          [
            [floorMins, "floor"],
            [targetMins, "goal"],
          ].map(
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
          const segs = parts(d).filter((s) => s.value > 0);
          const dim = hover === null || hover === i ? 1 : 0.55;
          let acc = 0;
          return html`
            ${segs.map((s, k) => {
              const yBase = y(acc);
              acc += s.value;
              const yTop = y(acc);
              const isTop = k === segs.length - 1;
              // 2px of surface between segments, taken off the top of every
              // segment but the last so the gaps sit *between* the fills.
              const h = yBase - yTop - (isTop ? 0 : 2);
              return (
                h > 0.5 &&
                html`
                  <path
                    d=${barPath(cx - barW / 2, yTop, barW, h, isTop ? 4 : 0)}
                    fill=${s.color}
                    opacity=${dim}
                  />
                `
              );
            })}
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
        split &&
        html`
          <div class="chart-legend">
            ${DAY_SPLIT.map(
            (s) => html`
              <span class="legend-item legend-static">
                <span
                  class="legend-swatch"
                  style=${`background:${s.color}`}
                ></span
                >${s.label}
              </span>
            `,
          )}
          </div>
        `
      }
      ${
        hover !== null &&
        html`
          <${Tooltip} x=${m.left + band * hover + band / 2} y=${8}>
            <${DayBarTooltip}
              day=${days[hover]}
              parts=${parts(days[hover])}
              split=${split}
              isMins=${isMins}
            />
          <//>
        `
      }
    </div>
  `;
}

function DayBarTooltip({ day, parts, split, isMins }) {
  const fmt = (v) =>
    isMins
      ? `${Math.round(v)} min`
      : `${Math.round(v).toLocaleString("en")} chars`;
  const headline = `${Math.round(day.active_secs / 60)} min · ${day.chars.toLocaleString("en")} chars`;
  return html`
    <strong>${day.date}</strong><br />
    ${headline}
    ${
      split &&
      parts
        .filter((s) => s.value > 0)
        .map((s) => {
          const line = `${s.label} ${fmt(s.value)}`;
          return html`<br /><span class="tooltip-sub">${line}</span>`;
        })
    }
  `;
}

/** Candidate y-axis steps for the speed chart, finest first. */
