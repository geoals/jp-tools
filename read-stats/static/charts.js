import { useState } from 'preact/hooks';
import { html } from 'htm/preact';

// Hand-rolled SVG charts following the dataviz mark specs: thin marks, 4px
// rounded data-ends (square at the baseline), 2px lines, hairline solid grid,
// text in ink tokens (never the series color), hover tooltip on every mark.

const W = 640;

function niceCeil(v, step) {
  return Math.max(step, Math.ceil(v / step) * step);
}

/** Rounded top corners only — bars stay square at the baseline. */
function barPath(x, y, w, h, r) {
  r = Math.min(r, h, w / 2);
  return `M${x},${y + h} L${x},${y + r} Q${x},${y} ${x + r},${y}`
    + ` L${x + w - r},${y} Q${x + w},${y} ${x + w},${y + r}`
    + ` L${x + w},${y + h} Z`;
}

function shortDate(iso) {
  const [, m, d] = iso.split('-');
  return `${Number(m)}/${Number(d)}`;
}

function Tooltip({ x, y, children }) {
  return html`
    <div class="chart-tooltip" style="left:${(x / W) * 100}%; top:${y}px">
      ${children}
    </div>
  `;
}

/** Daily reading minutes with goal reference lines. days: [{date, active_secs, chars}] */
export function MinutesBarChart({ days, floorMins, targetMins }) {
  const [hover, setHover] = useState(null);
  const H = 230;
  // Right margin holds the "goal 120" / "floor 60" labels — wide enough that
  // three-digit goals don't run off the viewBox.
  const m = { top: 16, right: 56, bottom: 24, left: 40 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  const minutes = days.map((d) => d.active_secs / 60);
  const yStep = targetMins >= 120 ? 60 : 30;
  const yMax = niceCeil(Math.max(...minutes, targetMins * 1.15), yStep);
  const y = (v) => m.top + plotH - (v / yMax) * plotH;

  const band = plotW / days.length;
  const barW = Math.min(24, band * 0.7);

  const ticks = [];
  for (let t = 0; t <= yMax; t += yStep) ticks.push(t);
  const labelEvery = Math.ceil(days.length / 7);

  if (!days.some((d) => d.active_secs > 0)) {
    return html`<p class="chart-empty">No reading recorded yet.</p>`;
  }

  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg viewBox="0 0 ${W} ${H}" role="img" aria-label="Daily reading minutes, last ${days.length} days">
        ${ticks.map((t) => html`
          <line x1=${m.left} x2=${W - m.right} y1=${y(t)} y2=${y(t)} class="gridline" />
          <text x=${m.left - 6} y=${y(t) + 3} class="tick" text-anchor="end">${t}</text>
        `)}
        ${[[floorMins, 'floor'], [targetMins, 'goal']].map(([v, name]) => html`
          <line x1=${m.left} x2=${W - m.right} y1=${y(v)} y2=${y(v)} class="goal-line" />
          <text x=${W - m.right + 4} y=${y(v) + 3} class="tick">${name} ${v}</text>
        `)}
        ${days.map((d, i) => {
          const mins = d.active_secs / 60;
          const cx = m.left + band * i + band / 2;
          return html`
            ${mins > 0.5 && html`
              <path d=${barPath(cx - barW / 2, y(mins), barW, y(0) - y(mins), 4)}
                    fill="var(--series-1)" opacity=${hover === null || hover === i ? 1 : 0.55} />
            `}
            ${i % labelEvery === 0 && html`
              <text x=${cx} y=${H - 8} class="tick" text-anchor="middle">${shortDate(d.date)}</text>
            `}
            <rect x=${m.left + band * i} y=${m.top} width=${band} height=${plotH}
                  fill="transparent" onMouseEnter=${() => setHover(i)} />
          `;
        })}
        <line x1=${m.left} x2=${W - m.right} y1=${y(0)} y2=${y(0)} class="baseline" />
      </svg>
      ${hover !== null && html`
        <${Tooltip} x=${m.left + band * hover + band / 2} y=${8}>
          <strong>${days[hover].date}</strong><br />
          ${Math.round(days[hover].active_secs / 60)} min · ${days[hover].chars.toLocaleString('en')} chars
        <//>
      `}
    </div>
  `;
}

/** Candidate y-axis steps for the speed chart, finest first. */
const SPEED_STEPS = [500, 1000, 2000, 2500, 5000, 10000];

/** Chars/hour trend over days with ≥10 min read. days: [{date, active_secs, chars}] */
export function SpeedTrendChart({ days }) {
  const [hover, setHover] = useState(null);
  const H = 210;
  const m = { top: 16, right: 56, bottom: 24, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  const points = days
    .map((d, i) => ({ ...d, i, speed: d.active_secs >= 600 ? d.chars / (d.active_secs / 3600) : null }))
    .filter((d) => d.speed !== null && d.speed > 0);

  if (points.length < 2) {
    return html`<p class="chart-empty">Needs a few days with 10+ minutes read to draw a trend.</p>`;
  }

  const rawMax = Math.max(...points.map((p) => p.speed));
  const rawMin = Math.min(...points.map((p) => p.speed));
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
  const x = (i) => m.left + (days.length === 1 ? 0 : (i / (days.length - 1)) * plotW);
  const y = (v) => m.top + plotH - ((v - yMin) / (yMax - yMin)) * plotH;

  // A sub-1k step needs a decimal, or 12500 and 13000 both label as "13k".
  const kLabel = (t) => `${(t / 1000).toFixed(yStep < 1000 ? 1 : 0)}k`;
  const ticks = [];
  for (let t = yMin; t <= yMax; t += yStep) ticks.push(t);
  const path = points.map((p, k) => `${k === 0 ? 'M' : 'L'}${x(p.i)},${y(p.speed)}`).join(' ');
  const last = points[points.length - 1];
  const labelEvery = Math.ceil(days.length / 6);

  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg viewBox="0 0 ${W} ${H}" role="img" aria-label="Reading speed trend, characters per hour">
        ${ticks.map((t) => html`
          <line x1=${m.left} x2=${W - m.right} y1=${y(t)} y2=${y(t)} class="gridline" />
          <text x=${m.left - 6} y=${y(t) + 3} class="tick" text-anchor="end">${kLabel(t)}</text>
        `)}
        ${days.map((d, i) => i % labelEvery === 0 && html`
          <text x=${x(i)} y=${H - 8} class="tick" text-anchor="middle">${shortDate(d.date)}</text>
        `)}
        <path d=${path} class="trend-line" />
        ${hover !== null && html`
          <line x1=${x(points[hover].i)} x2=${x(points[hover].i)} y1=${m.top} y2=${m.top + plotH} class="crosshair" />
        `}
        ${points.map((p, k) => html`
          <circle cx=${x(p.i)} cy=${y(p.speed)} r=${k === points.length - 1 || hover === k ? 5 : 3.5}
                  class="trend-dot" />
        `)}
        <text x=${x(last.i) + 10} y=${y(last.speed) + 4} class="end-label">
          ${(last.speed / 1000).toFixed(1)}k/h
        </text>
        <rect x=${m.left} y=${m.top} width=${plotW} height=${plotH} fill="transparent"
              onMouseMove=${(e) => {
                const rect = e.currentTarget.closest('svg').getBoundingClientRect();
                const px = ((e.clientX - rect.left) / rect.width) * W;
                let nearest = 0;
                points.forEach((p, k) => {
                  if (Math.abs(x(p.i) - px) < Math.abs(x(points[nearest].i) - px)) nearest = k;
                });
                setHover(nearest);
              }} />
      </svg>
      ${hover !== null && html`
        <${Tooltip} x=${x(points[hover].i)} y=${8}>
          <strong>${points[hover].date}</strong><br />
          ${Math.round(points[hover].speed).toLocaleString('en')} chars/h
          · ${Math.round(points[hover].active_secs / 60)} min
        <//>
      `}
    </div>
  `;
}

// Both series are events per hour, so they share one y-axis. Minutes read is a
// different unit and stays in its own chart — overlaying it here would mean two
// y-scales, whose alignment is arbitrary and invents correlations.
const RATE_SERIES = [
  { key: 'lookups', label: 'lookups/h', color: 'var(--series-1)', of: (d) => d.lookups },
  { key: 'cards', label: 'cards/h', color: 'var(--series-2)', of: (d) => d.cards },
];

/** Minimum active time before a per-hour rate means anything. */
const RATE_MIN_SECS = 600;

function rateStep(max) {
  if (max <= 10) return 2;
  if (max <= 30) return 5;
  if (max <= 60) return 10;
  return 20;
}

/**
 * Lookups and mined cards per hour of reading, toggleable.
 * days: [{date, active_secs, lookups, cards}]
 */
export function RateTrendChart({ days }) {
  const [hover, setHover] = useState(null);
  const [off, setOff] = useState({});
  const H = 210;
  const m = { top: 16, right: 64, bottom: 24, left: 44 };
  const plotW = W - m.left - m.right;
  const plotH = H - m.top - m.bottom;

  // Rate is per *hour of reading*, so a day is only a data point once it has
  // enough active time for the denominator to be stable.
  const rated = days
    .map((d, i) => ({ ...d, i, hours: d.active_secs / 3600 }))
    .filter((d) => d.active_secs >= RATE_MIN_SECS);

  const shown = RATE_SERIES.filter((s) => !off[s.key]);

  if (rated.length < 2) {
    return html`<p class="chart-empty">Needs a few days with 10+ minutes read to draw a trend.</p>`;
  }

  const values = shown.flatMap((s) => rated.map((d) => s.of(d) / d.hours));
  const step = rateStep(Math.max(...values, 1));
  const yMax = niceCeil(Math.max(...values, 1) * 1.1, step);
  const x = (i) => m.left + (days.length === 1 ? 0 : (i / (days.length - 1)) * plotW);
  const y = (v) => m.top + plotH - (v / yMax) * plotH;

  const ticks = [];
  for (let t = 0; t <= yMax; t += step) ticks.push(t);
  const labelEvery = Math.ceil(days.length / 6);

  const plotted = shown.map((s) => {
    const pts = rated.map((d) => ({ i: d.i, v: s.of(d) / d.hours }));
    return {
      s,
      pts,
      last: pts[pts.length - 1],
      path: pts.map((p, k) => `${k === 0 ? 'M' : 'L'}${x(p.i)},${y(p.v)}`).join(' '),
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
      ${RATE_SERIES.map((s) => html`
        <button type="button"
                class=${`legend-item${off[s.key] ? ' legend-off' : ''}`}
                aria-pressed=${!off[s.key]}
                onClick=${() => setOff((o) => ({ ...o, [s.key]: !o[s.key] }))}>
          <span class="legend-swatch" style=${`background:${s.color}`}></span>${s.label}
        </button>
      `)}
    </div>
  `;

  if (shown.length === 0) {
    return html`${legend}<p class="chart-empty">Both series hidden — pick one above.</p>`;
  }

  return html`
    ${legend}
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg viewBox="0 0 ${W} ${H}" role="img"
           aria-label="Lookups and mined cards per hour of reading, last ${days.length} days">
        ${ticks.map((t) => html`
          <line x1=${m.left} x2=${W - m.right} y1=${y(t)} y2=${y(t)} class="gridline" />
          <text x=${m.left - 6} y=${y(t) + 3} class="tick" text-anchor="end">${t}</text>
        `)}
        ${days.map((d, i) => i % labelEvery === 0 && html`
          <text x=${x(i)} y=${H - 8} class="tick" text-anchor="middle">${shortDate(d.date)}</text>
        `)}
        ${hover !== null && html`
          <line x1=${x(rated[hover].i)} x2=${x(rated[hover].i)} y1=${m.top} y2=${m.top + plotH}
                class="crosshair" />
        `}
        ${plotted.map(({ s, pts, path }) => html`
          <path d=${path} class="trend-line" style=${`stroke:${s.color}`} />
          ${pts.map((p, k) => html`
            <circle cx=${x(p.i)} cy=${y(p.v)} r=${k === pts.length - 1 || hover === k ? 5 : 3.5}
                    class="trend-dot" style=${`fill:${s.color}`} />
          `)}
        `)}
        ${labelled.map(({ s, last, labelY }) => html`
          <text x=${x(last.i) + 8} y=${labelY} class="end-label">${last.v.toFixed(1)}</text>
        `)}
        <line x1=${m.left} x2=${W - m.right} y1=${y(0)} y2=${y(0)} class="baseline" />
        <rect x=${m.left} y=${m.top} width=${plotW} height=${plotH} fill="transparent"
              onMouseMove=${(e) => {
                const rect = e.currentTarget.closest('svg').getBoundingClientRect();
                const px = ((e.clientX - rect.left) / rect.width) * W;
                let nearest = 0;
                rated.forEach((d, k) => {
                  if (Math.abs(x(d.i) - px) < Math.abs(x(rated[nearest].i) - px)) nearest = k;
                });
                setHover(nearest);
              }} />
      </svg>
      ${hover !== null && html`
        <${Tooltip} x=${x(rated[hover].i)} y=${8}>
          <strong>${rated[hover].date}</strong><br />
          ${shown.map((s) => html`
            ${s.label.replace('/h', '')}: ${(s.of(rated[hover]) / rated[hover].hours).toFixed(1)}/h<br />
          `)}
          <span class="tooltip-sub">${Math.round(rated[hover].active_secs / 60)} min read</span>
        <//>
      `}
    </div>
  `;
}

/** Plain progress bar (same visual language as the goal meter, no marker). */
export function ProgressBar({ pct, label }) {
  return html`
    <div class="meter" role="meter" aria-valuenow=${Math.round(pct)} aria-valuemin="0"
         aria-valuemax="100" aria-label=${label}>
      <div class="meter-fill" style="width:${Math.min(100, pct)}%"></div>
    </div>
  `;
}

/** Goal meter: fill in the series hue, unfilled track a lighter step of the same ramp. */
export function GoalMeter({ mins, floorMins, targetMins }) {
  const pct = Math.min(100, (mins / targetMins) * 100);
  const floorPct = Math.min(100, (floorMins / targetMins) * 100);
  return html`
    <div class="meter" role="meter" aria-valuenow=${Math.round(mins)} aria-valuemin="0"
         aria-valuemax=${targetMins} aria-label="Minutes read toward ${targetMins}-minute target">
      <div class="meter-fill" style="width:${pct}%"></div>
      <div class="meter-marker" style="left:${floorPct}%" title="floor ${floorMins} min"></div>
    </div>
  `;
}
