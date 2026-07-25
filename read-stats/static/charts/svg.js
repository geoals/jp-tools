// The primitives every chart is drawn from: the fixed viewBox width, axis
// rounding, the rounded-end bar path, the shared tooltip, and the tick and
// segment helpers.
//
// The charts are hand-rolled SVG rather than a library because the mark specs
// are opinionated in ways charting libraries fight — thin marks, 4px rounded
// data-ends but square at the baseline, hairline solid grid, and text in ink
// tokens rather than the series colour. Keeping the primitives in one file is
// what makes five charts look like one system.

import { html } from "htm/preact";

export const W = 640;

export function niceCeil(v, step) {
  return Math.max(step, Math.ceil(v / step) * step);
}

/** Rounded top corners only — bars stay square at the baseline. */

export function barPath(x, y, w, h, r) {
  r = Math.min(r, h, w / 2);
  return (
    `M${x},${y + h} L${x},${y + r} Q${x},${y} ${x + r},${y}` +
    ` L${x + w - r},${y} Q${x + w},${y} ${x + w},${y + r}` +
    ` L${x + w},${y + h} Z`
  );
}

export function shortDate(iso) {
  const [, m, d] = iso.split("-");
  return `${Number(m)}/${Number(d)}`;
}

export function Tooltip({ x, y, children }) {
  return html`
    <div class="chart-tooltip" style="left:${(x / W) * 100}%; top:${y}px">
      ${children}
    </div>
  `;
}

/** Compact axis label for character counts: 20000 → "20k". */

export function kChars(n) {
  return n >= 1000 ? `${+(n / 1000).toFixed(n < 10000 ? 1 : 0)}k` : String(n);
}

/* The three segments a day's bar can split into. Dialogue and narration keep
   the hues they carry on the dialogue card — colour follows the entity, so the
   same green means narration wherever it appears.

   "no line text" is the remainder, and it is a real category rather than a
   rounding bucket: manually logged sessions have no hooked text to classify,
   so a day of physical-book reading is legitimately all remainder. Drawing it
   in muted ink rather than a fourth hue says that — it is the absence of the
   measurement, not a third kind of reading. */

export function truncWork(title, n = 10) {
  return title.length > n ? `${title.slice(0, n)}…` : title;
}

/** Days where the dominant work changed from the previous reading day — the
 *  points a chart marks so a speed step reads as "switched VN", not a slump.
 *  The first work to appear isn't a switch, so it gets no marker. A day with no
 *  reading (no `work`) is skipped, never treated as a change. */

export function rateStep(max) {
  if (max <= 10) return 2;
  if (max <= 30) return 5;
  if (max <= 60) return 10;
  return 20;
}

/**
 * Lookups and mined cards per hour of reading, toggleable.
 * days: [{date, active_secs, lookups, cards}]
 */

export function clockHM(ts) {
  const d = new Date(ts * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/**
 * Centred rolling window over the raw buckets, never crossing a session
 * boundary. Rates are a ratio of sums (total chars ÷ total seconds), not a mean
 * of per-bucket rates — averaging ratios would weight a 4-second bucket the
 * same as a full minute and let the quiet edges of a session dominate.
 */

export function segments(pts, key) {
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

export function niceTicks(max, count) {
  const raw = max / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step =
    [1, 2, 2.5, 5, 10].map((s) => s * mag).find((s) => s >= raw) ?? mag * 10;
  const top = Math.ceil(max / step) * step;
  const ticks = [];
  for (let t = 0; t <= top + 1e-9; t += step) ticks.push(t);
  return { ticks, top };
}

export function bandPath(seg, x, y) {
  const up = seg
    .map((p, k) => `${k === 0 ? "M" : "L"}${x(p.t)},${y(p.raw)}`)
    .join(" ");
  const back = [...seg]
    .reverse()
    .map((p) => `L${x(p.t)},${y(p.speed)}`)
    .join(" ");
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

export function workChanges(days) {
  const out = [];
  let prev = null;
  days.forEach((d, i) => {
    if (d.work && d.work !== prev) {
      if (prev !== null) out.push({ i, work: d.work });
      prev = d.work;
    }
  });
  return out;
}
