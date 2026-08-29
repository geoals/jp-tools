// The primitives every chart is drawn from.
//
// Hand-rolled SVG rather than a charting library: the mark specs are opinionated
// in ways a library fights — thin marks, 4px rounded data-ends but square at the
// baseline, hairline solid grid, text in ink tokens rather than the series
// colour. One file for the primitives is what makes the charts look like one
// system.

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

/** VN titles are long; a divider label only needs enough to recognise it. */

export function truncWork(title, n = 10) {
  return title.length > n ? `${title.slice(0, n)}…` : title;
}

export function rateStep(max) {
  if (max <= 10) return 2;
  if (max <= 30) return 5;
  if (max <= 60) return 10;
  return 20;
}

export function clockHM(ts) {
  const d = new Date(ts * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

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

/** Area between the two speed lines — the lookup tax, in chars/hour. */

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

/** Days where the dominant work changed from the previous reading day — the
 *  points a chart marks so a speed step reads as "switched VN", not a slump.
 *  The first work to appear isn't a switch, so it gets no marker. A day with no
 *  reading (no `work`) is skipped, never treated as a change. */

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
