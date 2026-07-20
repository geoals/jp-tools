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
        <path d=${path} class="trend-line trend-line-speed" />
        ${hover !== null && html`
          <line x1=${x(points[hover].i)} x2=${x(points[hover].i)} y1=${m.top} y2=${m.top + plotH} class="crosshair" />
        `}
        ${points.map((p, k) => html`
          <circle cx=${x(p.i)} cy=${y(p.speed)} r=${k === points.length - 1 || hover === k ? 5 : 3.5}
                  class="trend-dot trend-dot-speed" />
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

// ---------------------------------------------------------------------------
// Intra-day detail
// ---------------------------------------------------------------------------

/** Smoothed window needs at least this much reading time to report a rate. */
const DAY_MIN_ACTIVE_SECS = 45;

function clockHM(ts) {
  const d = new Date(ts * 1000);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/**
 * Centred rolling window over the raw buckets, never crossing a session
 * boundary. Rates are a ratio of sums (total chars ÷ total seconds), not a mean
 * of per-bucket rates — averaging ratios would weight a 4-second bucket the
 * same as a full minute and let the quiet edges of a session dominate.
 */
function smoothBuckets(buckets, win) {
  const half = Math.floor(win / 2);
  return buckets.map((b, i) => {
    let chars = 0, active = 0, lookup = 0, lookups = 0, cards = 0;
    const lo = Math.max(0, i - half);
    const hi = Math.min(buckets.length - 1, i + half);
    for (let j = lo; j <= hi; j += 1) {
      if (buckets[j].session !== b.session) continue;
      chars += buckets[j].chars;
      active += buckets[j].active_secs;
      lookup += buckets[j].lookup_secs;
      lookups += buckets[j].lookups;
      cards += buckets[j].cards;
    }
    const hours = active / 3600;
    const ok = active >= DAY_MIN_ACTIVE_SECS;
    // Speed on the text itself: the same characters, with the seconds that went
    // into lookups taken out of the denominator. Needs enough non-lookup time
    // left to divide by — a window that was *all* lookup has no raw speed.
    const textHours = (active - lookup) / 3600;
    return {
      t: b.t,
      session: b.session,
      winChars: chars,
      winActive: active,
      winLookup: lookup,
      speed: ok ? chars / hours : null,
      raw: ok && textHours > DAY_MIN_ACTIVE_SECS / 3600 ? chars / textHours : null,
      lookups: ok ? lookups / hours : null,
      cards: ok ? cards / hours : null,
    };
  });
}

/** Split points into drawable runs, breaking on a null value or a new session. */
function segments(pts, key) {
  const out = [];
  let cur = [];
  for (const p of pts) {
    const breaks = cur.length > 0 && cur[cur.length - 1].session !== p.session;
    if (p[key] === null || breaks) {
      if (cur.length > 1) out.push(cur);
      cur = [];
    }
    if (p[key] !== null) cur.push(p);
  }
  if (cur.length > 1) out.push(cur);
  return out;
}

function niceTicks(max, count) {
  const raw = max / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((s) => s * mag).find((s) => s >= raw) ?? mag * 10;
  const top = Math.ceil(max / step) * step;
  const ticks = [];
  for (let t = 0; t <= top + 1e-9; t += step) ticks.push(t);
  return { ticks, top };
}

const DAY_RATE_SERIES = [
  { key: 'lookups', label: 'lookups/h', color: 'var(--series-1)' },
  { key: 'cards', label: 'cards/h', color: 'var(--series-2)' },
];

/** Area between the two speed lines — the lookup tax, in chars/hour. */
function bandPath(seg, x, y) {
  const up = seg.map((p, k) => `${k === 0 ? 'M' : 'L'}${x(p.t)},${y(p.raw)}`).join(' ');
  const back = [...seg].reverse().map((p) => `L${x(p.t)},${y(p.speed)}`).join(' ');
  return `${up} ${back} Z`;
}

/**
 * One day's reading, minute by minute: speed above, lookup and mining rate
 * below, on a shared clock axis.
 *
 * Two panels rather than one overlay on purpose. Chars/hour runs in the
 * thousands and events/hour in the tens, so putting them on one plot would need
 * two y-scales — and where those two scales line up is a choice, not a fact, so
 * the picture would imply a correlation the data never stated. Stacked on a
 * shared x-axis, a dip in speed and a spike in lookups sit in the same vertical
 * slice and the comparison stays the reader's to make.
 */
export function DayTimelineChart({ buckets, bucketSecs, windowMins }) {
  const [hover, setHover] = useState(null);
  const [off, setOff] = useState({});
  const [overlay, setOverlay] = useState(false);

  // Right margin holds the two direct labels on the speed panel.
  const m = { top: 14, right: 82, bottom: 26, left: 48 };
  const aH = 132;   // speed panel
  const gap = 28;
  const bH = 92;    // rate panel
  const H = m.top + aH + gap + bH + m.bottom;
  const plotW = W - m.left - m.right;
  const aTop = m.top;
  const bTop = m.top + aH + gap;

  if (!buckets || buckets.length < 2) {
    return html`<p class="chart-empty">No line-stream reading recorded this day.</p>`;
  }

  const win = Math.max(1, Math.round((windowMins * 60) / bucketSecs));
  const pts = smoothBuckets(buckets, win);

  const t0 = buckets[0].t;
  const t1 = buckets[buckets.length - 1].t + bucketSecs;
  const x = (ts) => m.left + ((ts - t0) / (t1 - t0)) * plotW;

  const shown = DAY_RATE_SERIES.filter((s) => !off[s.key]);

  const speeds = pts.map((p) => p.speed).filter((v) => v !== null);
  if (speeds.length < 2) {
    return html`<p class="chart-empty">
      Not enough continuous reading this day to draw a curve — try a smaller smoothing window.
    </p>`;
  }
  const raws = pts.map((p) => p.raw).filter((v) => v !== null);
  const speedAxis = niceTicks(Math.max(...speeds, ...raws) * 1.1, 4);
  const rateVals = shown.flatMap((s) => pts.map((p) => p[s.key]).filter((v) => v !== null));
  const rateAxis = niceTicks(Math.max(...rateVals, 1) * 1.05, 4);

  const yA = (v) => aTop + aH - (v / speedAxis.top) * aH;
  const yB = (v) => bTop + bH - (v / rateAxis.top) * bH;

  const toPath = (segs, y, key) =>
    segs.map((seg) => seg.map((p, k) => `${k === 0 ? 'M' : 'L'}${x(p.t)},${y(p[key])}`).join(' '));

  const speedPaths = toPath(segments(pts, 'speed'), yA, 'speed');
  const rawPaths = toPath(segments(pts, 'raw'), yA, 'raw');
  // The band needs both lines defined, so mark the points where they overlap
  // and reuse the same segmenting.
  const paired = pts.map((p) => ({ ...p, both: p.raw !== null && p.speed !== null ? 1 : null }));
  const bands = segments(paired, 'both').map((seg) => bandPath(seg, x, yA));

  // Direct labels at each line's last defined point, nudged apart when the two
  // speeds finish close enough to overprint.
  const lastOf = (key) => [...pts].reverse().find((p) => p[key] !== null) ?? null;
  const lastSpeed = lastOf('speed');
  let lastRaw = lastOf('raw');
  if (lastSpeed && lastRaw && Math.abs(yA(lastRaw.raw) - yA(lastSpeed.speed)) < 13) {
    lastRaw = { ...lastRaw, raw: speedAxis.top * ((yA(lastSpeed.speed) - 13 - aTop - aH) / -aH) };
  }

  // Day-level tax, stated as a number rather than left to the eye.
  const totActive = buckets.reduce((a, b) => a + b.active_secs, 0);
  const totLookup = buckets.reduce((a, b) => a + b.lookup_secs, 0);
  const totChars = buckets.reduce((a, b) => a + b.chars, 0);
  const dayEffective = totChars / (totActive / 3600);
  const dayRaw = totActive > totLookup ? totChars / ((totActive - totLookup) / 3600) : null;

  // Hour gridlines, or half-hour when the day is short enough to need them.
  const spanHours = (t1 - t0) / 3600;
  const tickSecs = spanHours > 6 ? 7200 : spanHours > 3 ? 3600 : 1800;
  const timeTicks = [];
  for (let t = Math.ceil(t0 / tickSecs) * tickSecs; t < t1; t += tickSecs) timeTicks.push(t);

  // Panel A's two lines are one measure under two conditions, so they share a
  // hue and separate by dash + direct label; panel B's two are different
  // things and take the palette's two slots, as the 30-day chart already does.
  const legend = html`
    <div class="chart-legend">
      <span class="legend-item legend-static">
        <span class="legend-swatch legend-line"></span>as read
      </span>
      <span class="legend-item legend-static">
        <span class="legend-swatch legend-line legend-line-dashed"></span>lookups removed
      </span>
      ${DAY_RATE_SERIES.map((s) => html`
        <button type="button"
                class=${`legend-item${off[s.key] ? ' legend-off' : ''}`}
                aria-pressed=${!off[s.key]}
                onClick=${() => setOff((o) => ({ ...o, [s.key]: !o[s.key] }))}>
          <span class="legend-swatch" style=${`background:${s.color}`}></span>${s.label}
        </button>
      `)}
      <button type="button"
              class=${`legend-item${overlay ? '' : ' legend-off'}`}
              aria-pressed=${overlay}
              title="Draw the rate curves into the speed panel, each scaled 0–max. Shows when they move together; amplitude is normalised away, so read magnitude from the panel below or from hover."
              onClick=${() => setOverlay((v) => !v)}>
        ⇕ overlay shape
      </button>
    </div>
  `;

  // Overlay: each rate curve scaled 0→its own max over the visible window, by
  // a fixed rule rather than by eye. That makes the *timing* of a lookup spike
  // against a speed dip readable at a glance, and it is all it makes readable —
  // normalising to full height means every series looks equally variable
  // whatever its true swing. Magnitude stays with the panel below and the
  // tooltip, both of which report real per-hour values.
  const overlayPaths = !overlay
    ? []
    : shown.map((s) => {
        const peak = Math.max(...pts.map((p) => p[s.key] ?? 0), 1e-9);
        const yN = (v) => aTop + aH - (v / peak) * aH * 0.92;
        return {
          s,
          paths: toPath(segments(pts, s.key), yN, s.key),
        };
      });

  const hp = hover !== null ? pts[hover] : null;

  return html`
    <div class="chart-wrap" onMouseLeave=${() => setHover(null)}>
      <svg viewBox="0 0 ${W} ${H}" role="img"
           aria-label="Reading speed, lookup rate and mining rate across the day">
        <text x=${m.left} y=${aTop - 3} class="panel-title">chars/hour</text>
        ${speedAxis.ticks.map((t) => html`
          <line x1=${m.left} x2=${W - m.right} y1=${yA(t)} y2=${yA(t)} class="gridline" />
          <text x=${m.left - 6} y=${yA(t) + 3} class="tick" text-anchor="end">
            ${t >= 1000 ? `${(t / 1000).toFixed(t % 1000 ? 1 : 0)}k` : t}
          </text>
        `)}
        ${bands.map((d) => html`<path d=${d} class="tax-band" />`)}
        ${overlayPaths.map(({ s, paths }) => paths.map((d) => html`
          <path d=${d} class="overlay-line" style=${`stroke:${s.color}`} />
        `))}
        ${rawPaths.map((d) => html`<path d=${d} class="trend-line trend-line-speed trend-line-raw" />`)}
        ${speedPaths.map((d) => html`<path d=${d} class="trend-line trend-line-speed" />`)}
        ${lastSpeed && html`
          <text x=${x(lastSpeed.t) + 7} y=${yA(lastSpeed.speed) + 4} class="end-label">as read</text>
        `}
        ${lastRaw && html`
          <text x=${x(lastRaw.t) + 7} y=${yA(lastRaw.raw) + 4} class="end-label">no lookups</text>
        `}
        <line x1=${m.left} x2=${W - m.right} y1=${yA(0)} y2=${yA(0)} class="baseline" />

        <text x=${m.left} y=${bTop - 3} class="panel-title">per hour of reading</text>
        ${rateAxis.ticks.map((t) => html`
          <line x1=${m.left} x2=${W - m.right} y1=${yB(t)} y2=${yB(t)} class="gridline" />
          <text x=${m.left - 6} y=${yB(t) + 3} class="tick" text-anchor="end">${t}</text>
        `)}
        ${shown.map((s) => html`
          ${toPath(segments(pts, s.key), yB, s.key).map((d) => html`
            <path d=${d} class="trend-line" style=${`stroke:${s.color}`} />
          `)}
        `)}
        <line x1=${m.left} x2=${W - m.right} y1=${yB(0)} y2=${yB(0)} class="baseline" />

        ${timeTicks.map((t) => html`
          <text x=${x(t)} y=${H - 8} class="tick" text-anchor="middle">${clockHM(t)}</text>
        `)}
        ${hp && html`
          <line x1=${x(hp.t)} x2=${x(hp.t)} y1=${aTop} y2=${bTop + bH} class="crosshair" />
          ${hp.speed !== null && html`
            <circle cx=${x(hp.t)} cy=${yA(hp.speed)} r="4.5" class="trend-dot trend-dot-speed" />
          `}
          ${shown.map((s) => hp[s.key] !== null && html`
            <circle cx=${x(hp.t)} cy=${yB(hp[s.key])} r="4.5" class="trend-dot"
                    style=${`fill:${s.color}`} />
          `)}
        `}
        <rect x=${m.left} y=${aTop} width=${plotW} height=${bTop + bH - aTop} fill="transparent"
              onMouseMove=${(e) => {
                const rect = e.currentTarget.closest('svg').getBoundingClientRect();
                const px = ((e.clientX - rect.left) / rect.width) * W;
                let nearest = 0;
                pts.forEach((p, k) => {
                  if (Math.abs(x(p.t) - px) < Math.abs(x(pts[nearest].t) - px)) nearest = k;
                });
                setHover(nearest);
              }} />
      </svg>
      ${hp && html`
        <${Tooltip} x=${x(hp.t)} y=${6}>
          <strong>${clockHM(hp.t)}</strong><br />
          ${hp.speed === null
            ? html`<span class="tooltip-sub">too little reading in the window</span>`
            : html`
                ${`${Math.round(hp.speed).toLocaleString('en')} chars/h as read`}<br />
                ${hp.raw !== null &&
                  html`${`${Math.round(hp.raw).toLocaleString('en')} chars/h without lookups`}<br />`}
                ${shown.map((s) => html`
                  ${s.label.replace('/h', '')}: ${hp[s.key].toFixed(1)}/h<br />
                `)}
                <span class="tooltip-sub">
                  ${`${hp.winChars.toLocaleString('en')} chars · ${Math.round(hp.winActive / 60)} min read · ${Math.round(hp.winLookup / 60)} min of it lookups`}
                </span>
              `}
        <//>
      `}
    </div>
    ${legend}
    ${dayRaw !== null && html`
      <p class="chart-note">
        ${`Whole day: ${Math.round(dayEffective).toLocaleString('en')} chars/h as read, ${Math.round(dayRaw).toLocaleString('en')} without lookups — a lookup tax of ${Math.round(dayRaw - dayEffective).toLocaleString('en')} chars/h (${Math.round(((dayRaw - dayEffective) / dayRaw) * 100)}%), over ${Math.round(totLookup / 60)} min spent looking words up.`}
        ${' '}
        <span class="tooltip-sub">
          The 30s afk cap truncates long lookups, so this is a floor.
        </span>
      </p>
    `}
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
