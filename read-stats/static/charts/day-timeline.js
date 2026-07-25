// One day's reading curve at minute resolution: speed, lookups and cards
// against the clock, with the session bands behind them.
//
// Smoothing happens here rather than on the server, over buckets deliberately
// finer than anything worth plotting, so the granularity control is instant.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { Tooltip, W, bandPath, clockHM, niceTicks, segments } from "./svg.js";

const DAY_MIN_ACTIVE_SECS = 45;

function smoothBuckets(buckets, win) {
  const half = Math.floor(win / 2);
  return buckets.map((b, i) => {
    let chars = 0,
      cleanChars = 0,
      lookupChars = 0,
      active = 0,
      lookup = 0,
      lookups = 0,
      cards = 0;
    const lo = Math.max(0, i - half);
    const hi = Math.min(buckets.length - 1, i + half);
    for (let j = lo; j <= hi; j += 1) {
      if (buckets[j].session !== b.session) continue;
      chars += buckets[j].chars;
      cleanChars += buckets[j].clean_chars;
      lookupChars += buckets[j].lookup_chars;
      active += buckets[j].active_secs;
      lookup += buckets[j].lookup_secs;
      lookups += buckets[j].lookups;
      cards += buckets[j].cards;
    }
    const hours = active / 3600;
    const ok = active >= DAY_MIN_ACTIVE_SECS;
    // Speed on the text itself. Both sides drop together: characters read
    // across a lookup gap leave the numerator along with their seconds. Keeping
    // them while removing the seconds is what made a dense lookup burst report
    // 30k chars/h for reading that was really running at 12k.
    const textHours = (active - lookup) / 3600;
    // The two speeds have to be rates over the *same* characters, or the tax
    // between them is part accounting artefact. A session's trailing line has
    // no following gap, so it contributed no seconds to `active` — leaving it
    // in the effective numerator is free characters, and inflating effective
    // understates the tax. `clean + lookup` is exactly the timed chars.
    const timedChars = cleanChars + lookupChars;
    return {
      t: b.t,
      session: b.session,
      winChars: chars,
      winActive: active,
      winLookup: lookup,
      winCleanChars: cleanChars,
      // Time actually lost to the dictionary: the lookup gaps minus what the
      // lines inside them would have cost at clean pace.
      winOverhead: lookupOverhead(
        cleanChars,
        active - lookup,
        lookupChars,
        lookup,
      ),
      speed: ok ? timedChars / hours : null,
      raw:
        ok && textHours > DAY_MIN_ACTIVE_SECS / 3600
          ? cleanChars / textHours
          : null,
      lookups: ok ? lookups / hours : null,
      cards: ok ? cards / hours : null,
    };
  });
}

/**
 * Seconds genuinely lost to looking words up, given a window's clean and
 * lookup halves.
 *
 * A gap holding a lookup holds the line's reading too, so the whole gap is not
 * dictionary time. Price those characters at the window's uninterrupted pace
 * and subtract: on 2026-07-20 that turned "31 min looking words up" into 21.5,
 * the other 9 being reading that would have happened regardless.
 *
 * With no clean gaps in the window there is no pace to price them at, so the
 * whole of the lookup gaps is returned — knowingly an overstatement, since some
 * of it was still reading. That is the one window where the correction above
 * can't be applied, and it only arises when smoothing is tight enough for a
 * window to be nothing but lookups.
 */

function lookupOverhead(cleanChars, cleanSecs, lookupChars, lookupSecs) {
  if (cleanSecs <= 0 || cleanChars <= 0) return lookupSecs;
  const baseline = lookupChars / (cleanChars / cleanSecs);
  return Math.max(0, lookupSecs - baseline);
}

/** Split points into drawable runs, breaking on a null value or a new session. */

const DAY_RATE_SERIES = [
  { key: "lookups", label: "lookups/h", color: "var(--series-1)" },
  { key: "cards", label: "cards/h", color: "var(--series-2)" },
];

/** Area between the two speed lines — the lookup tax, in chars/hour. */

export function DayTimelineChart({ buckets, bucketSecs, windowMins }) {
  const [hover, setHover] = useState(null);
  const [off, setOff] = useState({});
  const [overlay, setOverlay] = useState(false);

  // Right margin holds the two direct labels on the speed panel.
  const m = { top: 16, right: 82, bottom: 30, left: 48 };
  const aH = 220; // speed panel
  const gap = 34;
  const bH = 150; // rate panel
  const H = m.top + aH + gap + bH + m.bottom;
  const plotW = W - m.left - m.right;
  const aTop = m.top;
  const bTop = m.top + aH + gap;

  if (!buckets || buckets.length < 2) {
    return html`<p class="chart-empty">
      No line-stream reading recorded this day.
    </p>`;
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
      Not enough continuous reading this day to draw a curve — try a smaller
      smoothing window.
    </p>`;
  }
  const raws = pts.map((p) => p.raw).filter((v) => v !== null);
  const speedAxis = niceTicks(Math.max(...speeds, ...raws) * 1.1, 6);
  const rateVals = shown.flatMap((s) =>
    pts.map((p) => p[s.key]).filter((v) => v !== null),
  );
  const rateAxis = niceTicks(Math.max(...rateVals, 1) * 1.05, 5);

  const yA = (v) => aTop + aH - (v / speedAxis.top) * aH;
  const yB = (v) => bTop + bH - (v / rateAxis.top) * bH;

  const toPath = (segs, y, key) =>
    segs.map((seg) =>
      seg
        .map((p, k) => `${k === 0 ? "M" : "L"}${x(p.t)},${y(p[key])}`)
        .join(" "),
    );

  const speedPaths = toPath(segments(pts, "speed"), yA, "speed");
  const rawPaths = toPath(segments(pts, "raw"), yA, "raw");
  // The band needs both lines defined, so mark the points where they overlap
  // and reuse the same segmenting.
  const paired = pts.map((p) => ({
    ...p,
    both: p.raw !== null && p.speed !== null ? 1 : null,
  }));
  const bands = segments(paired, "both").map((seg) => bandPath(seg, x, yA));

  // Direct labels at each line's last defined point, nudged apart when the two
  // speeds finish close enough to overprint.
  const lastOf = (key) =>
    [...pts].reverse().find((p) => p[key] !== null) ?? null;
  const lastSpeed = lastOf("speed");
  let lastRaw = lastOf("raw");
  if (
    lastSpeed &&
    lastRaw &&
    Math.abs(yA(lastRaw.raw) - yA(lastSpeed.speed)) < 13
  ) {
    lastRaw = {
      ...lastRaw,
      raw: speedAxis.top * ((yA(lastSpeed.speed) - 13 - aTop - aH) / -aH),
    };
  }

  // Day-level tax, stated as a number rather than left to the eye.
  const totActive = buckets.reduce((a, b) => a + b.active_secs, 0);
  const totLookup = buckets.reduce((a, b) => a + b.lookup_secs, 0);
  const totClean = buckets.reduce((a, b) => a + b.clean_chars, 0);
  const totLookupChars = buckets.reduce((a, b) => a + b.lookup_chars, 0);
  const dayOverhead = lookupOverhead(
    totClean,
    totActive - totLookup,
    totLookupChars,
    totLookup,
  );
  // Timed chars, not all chars — see `smoothBuckets`. Each session's trailing
  // line has chars but no credited seconds, and counting it here only would
  // make the tax a comparison between two different character sets.
  const dayEffective = (totClean + totLookupChars) / (totActive / 3600);
  const dayRaw =
    totActive > totLookup ? totClean / ((totActive - totLookup) / 3600) : null;

  // Hour gridlines, or half-hour when the day is short enough to need them.
  const spanHours = (t1 - t0) / 3600;
  const tickSecs = spanHours > 6 ? 7200 : spanHours > 3 ? 3600 : 1800;
  const timeTicks = [];
  for (let t = Math.ceil(t0 / tickSecs) * tickSecs; t < t1; t += tickSecs)
    timeTicks.push(t);

  // Panel A's two lines are one measure under two conditions, so they share a
  // hue and separate by dash + direct label; panel B's two are different
  // things and take the palette's two slots, as the 30-day chart already does.
  const legend = html`
    <div class="chart-legend">
      <span class="legend-item legend-static">
        <span class="legend-swatch legend-line"></span>as read
      </span>
      <span class="legend-item legend-static">
        <span class="legend-swatch legend-line legend-line-dashed"></span
        >lookups removed
      </span>
      ${DAY_RATE_SERIES.map(
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
      <button
        type="button"
        class=${`legend-item${overlay ? "" : " legend-off"}`}
        aria-pressed=${overlay}
        title="Draw the rate curves into the speed panel, each scaled 0–max. Shows when they move together; amplitude is normalised away, so read magnitude from the panel below or from hover."
        onClick=${() => setOverlay((v) => !v)}
      >
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
      <svg
        viewBox="0 0 ${W} ${H}"
        role="img"
        aria-label="Reading speed, lookup rate and mining rate across the day"
      >
        <text x=${m.left} y=${aTop - 3} class="panel-title">chars/hour</text>
        ${speedAxis.ticks.map(
          (t) => html`
            <line
              x1=${m.left}
              x2=${W - m.right}
              y1=${yA(t)}
              y2=${yA(t)}
              class="gridline"
            />
            <text x=${m.left - 6} y=${yA(t) + 3} class="tick" text-anchor="end">
              ${t >= 1000 ? `${(t / 1000).toFixed(t % 1000 ? 1 : 0)}k` : t}
            </text>
          `,
        )}
        ${bands.map((d) => html`<path d=${d} class="tax-band" />`)}
        ${overlayPaths.map(({ s, paths }) =>
          paths.map(
            (d) => html`
              <path d=${d} class="overlay-line" style=${`stroke:${s.color}`} />
            `,
          ),
        )}
        ${rawPaths.map((d) => html`<path d=${d} class="trend-line trend-line-speed trend-line-raw" />`)}
        ${speedPaths.map((d) => html`<path d=${d} class="trend-line trend-line-speed" />`)}
        ${
          lastSpeed &&
          html`
            <text
              x=${x(lastSpeed.t) + 7}
              y=${yA(lastSpeed.speed) + 4}
              class="end-label"
              >as read</text
            >
          `
        }
        ${
          lastRaw &&
          html`
            <text
              x=${x(lastRaw.t) + 7}
              y=${yA(lastRaw.raw) + 4}
              class="end-label"
              >no lookups</text
            >
          `
        }
        <line
          x1=${m.left}
          x2=${W - m.right}
          y1=${yA(0)}
          y2=${yA(0)}
          class="baseline"
        />

        <text x=${m.left} y=${bTop - 3} class="panel-title">
          per hour of reading
        </text>
        ${rateAxis.ticks.map(
          (t) => html`
            <line
              x1=${m.left}
              x2=${W - m.right}
              y1=${yB(t)}
              y2=${yB(t)}
              class="gridline"
            />
            <text x=${m.left - 6} y=${yB(t) + 3} class="tick" text-anchor="end"
              >${t}</text
            >
          `,
        )}
        ${shown.map(
          (s) => html`
            ${toPath(segments(pts, s.key), yB, s.key).map(
            (d) => html`
              <path d=${d} class="trend-line" style=${`stroke:${s.color}`} />
            `,
          )}
          `,
        )}
        <line
          x1=${m.left}
          x2=${W - m.right}
          y1=${yB(0)}
          y2=${yB(0)}
          class="baseline"
        />

        ${timeTicks.map(
          (t) => html`
            <text x=${x(t)} y=${H - 8} class="tick" text-anchor="middle"
              >${clockHM(t)}</text
            >
          `,
        )}
        ${
          hp &&
          html`
            <line
              x1=${x(hp.t)}
              x2=${x(hp.t)}
              y1=${aTop}
              y2=${bTop + bH}
              class="crosshair"
            />
            ${
            hp.speed !== null &&
            html`
              <circle
                cx=${x(hp.t)}
                cy=${yA(hp.speed)}
                r="4.5"
                class="trend-dot trend-dot-speed"
              />
            `
          }
            ${shown.map(
            (s) =>
              hp[s.key] !== null &&
              html`
                <circle
                  cx=${x(hp.t)}
                  cy=${yB(hp[s.key])}
                  r="4.5"
                  class="trend-dot"
                  style=${`fill:${s.color}`}
                />
              `,
          )}
          `
        }
        <rect
          x=${m.left}
          y=${aTop}
          width=${plotW}
          height=${bTop + bH - aTop}
          fill="transparent"
          onMouseMove=${(e) => {
                const rect = e.currentTarget
                  .closest("svg")
                  .getBoundingClientRect();
                const px = ((e.clientX - rect.left) / rect.width) * W;
                let nearest = 0;
                pts.forEach((p, k) => {
                  if (Math.abs(x(p.t) - px) < Math.abs(x(pts[nearest].t) - px))
                    nearest = k;
                });
                setHover(nearest);
              }}
        />
      </svg>
      ${
        hp &&
        html`
          <${Tooltip} x=${x(hp.t)} y=${6}>
            <strong>${clockHM(hp.t)}</strong><br />
            ${
            hp.speed === null
              ? html`<span class="tooltip-sub"
                  >too little reading in the window</span
                >`
              : html`
                  ${`${Math.round(hp.speed).toLocaleString("en")} chars/h as read`}<br />
                  ${
                  hp.raw !== null &&
                  html`${`${Math.round(hp.raw).toLocaleString("en")} chars/h without lookups`}<br />`
                }
                  ${shown.map(
                  (s) => html`
                    ${s.label.replace("/h", "")}: ${hp[s.key].toFixed(1)}/h<br />
                  `,
                )}
                  <span class="tooltip-sub">
                    ${`${hp.winChars.toLocaleString("en")} chars · ${Math.round(hp.winActive / 60)} min read · ${Math.round(hp.winOverhead / 6) / 10} min lost to lookups`}
                  </span>
                `
          }
          <//>
        `
      }
    </div>
    ${legend}
    ${
      dayRaw !== null &&
      html`
        <p class="chart-note">
          ${`Whole day: ${Math.round(dayEffective).toLocaleString("en")} chars/h as read, ${Math.round(dayRaw).toLocaleString("en")} without lookups — a lookup tax of ${Math.round(dayRaw - dayEffective).toLocaleString("en")} chars/h (${Math.round(((dayRaw - dayEffective) / dayRaw) * 100)}%).`}
          ${" "}
          ${`Lookups cost about ${Math.round(dayOverhead / 60)} min: ${Math.round(totLookup / 60)} min sat in gaps holding one, but ${Math.round((totLookup - dayOverhead) / 60)} min of that was reading the line itself.`}
          ${" "}
          <span class="tooltip-sub">
            A lookup running past the 30s afk cap is only ever charged 30s, so
            this is a slight floor.
          </span>
        </p>
      `
    }
  `;
}

/** Plain progress bar (same visual language as the goal meter, no marker). */
