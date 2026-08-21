// The kanji tab: every kanji ever read, and a few readings of that one list.
//
// The grid is the card the others explain. A kanji's chip is tinted by how
// often it has been read — on a log scale, because linear would leave
// everything but 人事言 invisible.
//
// One payload feeds all of it (see routes/kanji.rs). Everything below is a
// re-slice in JS, which is what keeps the grid and the grade meters from
// disagreeing about a kanji met while the page was open.
//
// It is fetched here rather than in the dashboard's shared poll: the derivation
// walks every line ever read, and no other panel needs the result.

import { html } from "htm/preact";
import { useEffect, useMemo, useState } from "preact/hooks";
import { api } from "../api.js";
import { DiscoveryChart } from "../charts.js";
import { SegmentedControl } from "../components/controls.js";

const SORTS = [
  { value: "count", label: "most read" },
  { value: "bccwj", label: "most common" },
  { value: "recent", label: "newest" },
];

/** Grade 8 is the whole secondary-school half of the jōyō set, and 0 is
 *  everything no grade teaches — mostly the kanji VNs reach for. */
const GRADE_LABEL = {
  1: "grade 1",
  2: "grade 2",
  3: "grade 3",
  4: "grade 4",
  5: "grade 5",
  6: "grade 6",
  8: "secondary",
  0: "non-jōyō",
};

export function KanjiView() {
  const [kanji, setKanji] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    api("/api/kanji")
      .then(setKanji)
      .catch((e) => setError(e.message));
  }, []);

  if (error) return html`<p class="chart-empty">Failed to load: ${error}</p>`;
  if (!kanji) return html`<p class="chart-empty">Loading…</p>`;
  if (!kanji.kanji.length) {
    return html`
      <div class="card">
        <p class="chart-empty">
          No kanji read yet — the grid fills in from the line stream.
        </p>
      </div>
    `;
  }

  const rows = kanji.kanji;
  const solidAt = kanji.solid_encounters;
  const joyo = kanji.grades.filter((g) => g.grade !== 0);
  const joyoMet = joyo.reduce((n, g) => n + g.met, 0);
  const joyoTotal = joyo.reduce((n, g) => n + g.total, 0);
  const solid = rows.filter((r) => r.count >= solidAt).length;
  const once = rows.filter((r) => r.count === 1).length;
  // Built whole, leading space included: htm collapses the whitespace where a
  // literal and an interpolation straddle a line break.
  const joyoOf = ` (${joyoMet} / ${joyoTotal})`;
  const solidOf = ` (≥${solidAt}×)`;

  return html`
    <div class="card">
      <h2>Kanji</h2>
      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">distinct kanji</div>
          <div class="value">${rows.length.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">jōyō met</div>
          <div class="value">
            ${Math.round((joyoMet / joyoTotal) * 100)}%
            <span class="value-sub">${joyoOf}</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">solid</div>
          <div class="value">
            ${solid.toLocaleString("en")}
            <span class="value-sub">${solidOf}</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">seen once</div>
          <div class="value">${once.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">kanji read</div>
          <div class="value">
            ${kanji.total_encounters.toLocaleString("en")}
          </div>
        </div>
      </div>
    </div>

    <${GridCard} rows=${rows} solidAt=${solidAt} />
    <${GradeCard} grades=${kanji.grades} solidAt=${solidAt} />

    <div class="card">
      <h2>New kanji per day</h2>
      <${DiscoveryChart}
        days=${kanji.days}
        label="Kanji met for the first time, per day"
        empty="No kanji read yet."
        barLabel="new that day"
        lineLabel="distinct kanji so far"
        tip=${KanjiDayTip}
      />
    </div>
  `;
}

/* The grid ---------------------------------------------------------------- */

/** Tint strength for a chip, on a log scale against the most-read kanji. The
 *  floor keeps a once-seen kanji visible as a chip rather than a hole in the
 *  grid, and the ceiling stops the top of the ramp from swallowing the glyph. */
function tint(count, max) {
  const t = Math.log(count) / Math.log(max);
  return 0.08 + 0.5 * t;
}

function GridCard({ rows, solidAt }) {
  const [sort, setSort] = useState("count");
  const [picked, setPicked] = useState(null);

  const max = rows[0].count;
  const shown = useMemo(() => {
    const by = {
      // Kanji BCCWJ never saw sort last, after every ranked one — "unlisted"
      // is rarer than rank 6937, not commoner than rank 1.
      bccwj: (a, b) =>
        (a.bccwj_rank ?? Infinity) - (b.bccwj_rank ?? Infinity) ||
        b.count - a.count,
      recent: (a, b) => b.first_ts - a.first_ts,
      count: (a, b) => b.count - a.count,
    }[sort];
    return [...rows].sort(by);
  }, [rows, sort]);

  const detail = picked && shown.find((r) => r.kanji === picked);
  const countLine = `${shown.length.toLocaleString("en")} kanji`;

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Every kanji read</h2>
        <span class="kanji-count">${countLine}</span>
      </div>
      <div class="card-controls kanji-sort">
        <span class="kanji-sort-label" id="kanji-sort-label">Order by</span>
        <${SegmentedControl}
          value=${sort}
          options=${SORTS}
          onChange=${setSort}
          label="Order the kanji grid by"
        />
      </div>

      <div class="kanji-grid">
        ${shown.map((r) => {
          const classes = ["kanji-cell"];
          if (r.kanji === picked) classes.push("kanji-picked");
          if (r.count >= solidAt) classes.push("kanji-solid");
          const rank = r.bccwj_rank ? `BCCWJ #${r.bccwj_rank}` : "not in BCCWJ";
          const card = r.mined ? " · on a card" : "";
          const title = `${r.kanji} · read ${r.count}× · ${rank}${card}`;
          return html`
            <button
              type="button"
              class=${classes.join(" ")}
              style=${`--k:${tint(r.count, max)}`}
              title=${title}
              onClick=${() => setPicked(r.kanji === picked ? null : r.kanji)}
            >
              ${r.kanji}
            </button>
          `;
        })}
      </div>

      ${detail ? html`<${Inspector} row=${detail} />` : html`<${GridLegend} />`}
    </div>
  `;
}

function GridLegend() {
  return html`
    <div class="chart-legend kanji-legend">
      ${[0.12, 0.3, 0.45, 0.58].map(
        (k) =>
          html`<span
            class="legend-swatch kanji-swatch"
            style=${`--k:${k}`}
          ></span>`,
      )}
      <span class="legend-item legend-static">rarely → constantly read</span>
    </div>
  `;
}

function Inspector({ row }) {
  const met = new Date(row.first_ts * 1000).toISOString().slice(0, 10);
  const last = new Date(row.last_ts * 1000).toISOString().slice(0, 10);
  const seen = `${row.count.toLocaleString("en")}× over ${row.days} day${row.days === 1 ? "" : "s"}`;
  const grade = GRADE_LABEL[row.grade ?? 0];
  const strokes = row.strokes ? `${row.strokes} strokes` : "";
  // BCCWJ ranks the whole tail, so it can say "rare" where the newspaper list
  // can only say "unlisted" — which is why it leads and the other follows.
  const bccwj = row.bccwj_rank
    ? `BCCWJ #${row.bccwj_rank.toLocaleString("en")}`
    : "not once in BCCWJ";
  const share = row.bccwj_per_million
    ? `${row.bccwj_per_million.toFixed(1)} per million`
    : "";
  const first = `first met ${met}, last ${last}`;
  const looked =
    row.lookups > 0
      ? `${row.lookups} lookup${row.lookups === 1 ? "" : "s"} on words with it`
      : "never looked up";
  const card = row.mined ? "on a card" : "no card";
  return html`
    <div class="kanji-detail">
      <div class="kanji-detail-glyph" style="--k:0.5">${row.kanji}</div>
      <div class="kanji-detail-body">
        <div class="kanji-detail-head">
          <strong>${row.gloss || "—"}</strong>
          <span class="kanji-detail-meta">
            ${[grade, strokes, bccwj, share].filter(Boolean).join(" · ")}
          </span>
        </div>
        <div class="kanji-detail-meta">${seen} · ${first}</div>
        <div class="kanji-detail-meta">${looked} · ${card}</div>
        ${
          row.top_work &&
          html`<div class="kanji-detail-meta">mostly in ${row.top_work}</div>`
        }
        ${
          row.words.length > 0 &&
          html`<div class="word-chips">
            ${row.words.map((w) => html`<span class="chip">${w}</span>`)}
          </div>`
        }
      </div>
    </div>
  `;
}

/* Grade coverage ---------------------------------------------------------- */

function GradeCard({ grades, solidAt }) {
  const joyo = grades.filter((g) => g.grade !== 0);
  const other = grades.find((g) => g.grade === 0);
  return html`
    <div class="card">
      <h2>Jōyō coverage</h2>
      <div class="compare">
        ${joyo.map((g) => {
          const metPct = (g.met / g.total) * 100;
          const solidPct = (g.solid / g.total) * 100;
          const value = `${g.met} / ${g.total}`;
          return html`
            <div class="compare-row">
              <span class="compare-name">${GRADE_LABEL[g.grade]}</span>
              <span class="compare-track">
                <span
                  class="compare-fill grade-met"
                  style=${`width:${metPct}%`}
                >
                  <span
                    class="compare-fill grade-solid"
                    style=${`width:${g.met ? (solidPct / metPct) * 100 : 0}%`}
                  ></span>
                </span>
              </span>
              <span class="compare-value">${value}</span>
            </div>
          `;
        })}
        ${
          // No grade teaches these, so there is no denominator to draw against.
          other &&
          html`<div
            class="compare-row"
            title="Jinmeiyō names and the hyōgai the writing reaches for — no grade teaches them, so there is nothing to draw a bar against."
          >
            <span class="compare-name">non-jōyō</span>
            <span class="compare-track"></span>
            <span class="compare-value">${other.met.toLocaleString("en")}</span>
          </div>`
        }
      </div>
      <div class="chart-legend kanji-legend">
        <span class="legend-item legend-static">
          <span class="legend-swatch" style="background:var(--series-1)"></span
          >${`solid (≥${solidAt}×)`}
        </span>
        <span class="legend-item legend-static">
          <span
            class="legend-swatch"
            style="background:var(--meter-track)"
          ></span
          >met at least once
        </span>
      </div>
    </div>
  `;
}

function KanjiDayTip({ day }) {
  const newLine = `${day.new} new kanji`;
  const totalLine = `${day.cumulative.toLocaleString("en")} distinct so far`;
  const encLine = `${day.encounters.toLocaleString("en")} kanji read`;
  return html`
    <strong>${day.date}</strong><br />
    ${newLine}<br />
    <span class="tooltip-sub">${totalLine}</span><br />
    <span class="tooltip-sub">${encLine}</span>
  `;
}
