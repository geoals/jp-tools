// The kanji tab: every kanji ever read, and six readings of that one list.
//
// The grid is the card the others explain. A kanji's chip is tinted by how
// often it has been read — on a log scale, because linear would leave
// everything but 人事言 invisible — and ringed twice over: green when the
// target word of a card in the deck contains it, red when it has been looked up
// out of all proportion to how often it was met.
//
// Both rings are deliberately rare. A ring on every kanji ever looked up says
// nothing — one lookup is what reading is — so the red ring is a threshold on
// the lookup rate against your own average (routes/kanji.rs decides it), not a
// mark for "has been looked up at all".
//
// One payload feeds all of it (see routes/kanji.rs). Everything below is a
// re-slice in JS, which is what keeps the grid and the coverage meters from
// disagreeing about a kanji met while the page was open.

import { html } from "htm/preact";
import { useMemo, useState } from "preact/hooks";
import { KanjiCoverageChart, KanjiDiscoveryChart } from "../charts.js";
import { SegmentedControl } from "../components/controls.js";

const SORTS = [
  { value: "count", label: "yours" },
  { value: "bccwj", label: "BCCWJ" },
  { value: "grade", label: "grade" },
  { value: "recent", label: "newest" },
  { value: "hard", label: "lookup rate" },
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

const HARD_LEN = 15;
const GAP_LEN = 40;

export function KanjiView({ kanji }) {
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
  // The red ring's rule, as the server applied it.
  const rule = {
    baseline: kanji.baseline_lookup_rate,
    multiple: kanji.outlier_multiple,
    floor: kanji.outlier_encounters,
  };
  const joyo = kanji.grades.filter((g) => g.grade !== 0);
  const joyoMet = joyo.reduce((n, g) => n + g.met, 0);
  const joyoTotal = joyo.reduce((n, g) => n + g.total, 0);
  const solid = rows.filter((r) => r.count >= solidAt).length;
  const once = rows.filter((r) => r.count === 1).length;
  // Built whole, leading space included: htm collapses the whitespace where a
  // literal and an interpolation straddle a line break, and prettier decides
  // where those breaks go.
  const joyoOf = ` (${joyoMet} / ${joyoTotal})`;
  const solidOf = ` (≥${solidAt}×)`;

  return html`
    <div class="tile-row">
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
        <div class="value">${kanji.total_encounters.toLocaleString("en")}</div>
      </div>
    </div>

    <${GridCard} rows=${rows} solidAt=${solidAt} rule=${rule} />
    <${GradeCard} grades=${kanji.grades} solidAt=${solidAt} />

    <div class="card">
      <h2>Coverage of what you read</h2>
      <${KanjiCoverageChart}
        kanji=${rows}
        totalEncounters=${kanji.total_encounters}
      />
      <p class="chart-note">
        Every kanji you have read, commonest first, against the share of all
        kanji encounters it accounts for. The flat right-hand stretch is the
        tail: thousands of glyphs, each one worth a fraction of a percent, and
        the ones that still stop you mid-sentence.
      </p>
    </div>

    <div class="card">
      <h2>New kanji per day</h2>
      <${KanjiDiscoveryChart} days=${kanji.days} />
    </div>

    <${HardestCard} rows=${rows} rule=${rule} />
    <${GapCard} rows=${rows} solidAt=${solidAt} />
    <${WorksCard} works=${kanji.works} />
  `;
}

/* The grid ---------------------------------------------------------------- */

/** Tint strength for a chip, on a log scale against the commonest kanji. The
 *  floor keeps a once-seen kanji visible as a chip rather than a hole in the
 *  grid, and the ceiling stops the top of the ramp from swallowing the glyph. */
function tint(count, max) {
  const t = Math.log(count) / Math.log(max);
  return 0.08 + 0.5 * t;
}

function GridCard({ rows, solidAt, rule }) {
  const [sort, setSort] = useState("count");
  const [onlyUnmined, setOnlyUnmined] = useState(false);
  const [onlyStruggling, setOnlyStruggling] = useState(false);
  const [picked, setPicked] = useState(null);

  const max = rows[0].count;
  const shown = useMemo(() => {
    let list = rows;
    if (onlyUnmined) list = list.filter((r) => !r.mined);
    if (onlyStruggling) list = list.filter((r) => r.struggling);
    const by = {
      // Ungraded kanji sort last rather than first: 0 means "no grade", not
      // "easiest".
      grade: (a, b) => (a.grade ?? 99) - (b.grade ?? 99) || b.count - a.count,
      // Kanji BCCWJ never saw sort last, after every ranked one — "unlisted"
      // is rarer than rank 6937, not commoner than rank 1.
      bccwj: (a, b) =>
        (a.bccwj_rank ?? Infinity) - (b.bccwj_rank ?? Infinity) ||
        b.count - a.count,
      recent: (a, b) => b.first_ts - a.first_ts,
      hard: (a, b) =>
        b.lookups / Math.max(b.metered_count, 1) -
          a.lookups / Math.max(a.metered_count, 1) || b.count - a.count,
      count: (a, b) => b.count - a.count,
    }[sort];
    return [...list].sort(by);
  }, [rows, sort, onlyUnmined, onlyStruggling]);

  const detail = picked && shown.find((r) => r.kanji === picked);

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Every kanji you have read</h2>
        <div class="card-controls">
          <${SegmentedControl}
            value=${sort}
            options=${SORTS}
            onChange=${setSort}
            label="Sort the kanji grid"
          />
        </div>
      </div>
      <div class="card-controls kanji-filters">
        <button
          type="button"
          class=${onlyUnmined ? "toggle-btn toggle-on" : "toggle-btn"}
          aria-pressed=${onlyUnmined}
          onClick=${() => setOnlyUnmined(!onlyUnmined)}
        >
          no card yet
        </button>
        <button
          type="button"
          class=${onlyStruggling ? "toggle-btn toggle-on" : "toggle-btn"}
          aria-pressed=${onlyStruggling}
          onClick=${() => setOnlyStruggling(!onlyStruggling)}
        >
          struggling with
        </button>
        <span class="kanji-count"
          >${shown.length.toLocaleString("en")} shown</span
        >
      </div>

      <div class="kanji-grid">
        ${shown.map((r) => {
          const classes = ["kanji-cell"];
          if (r.struggling) classes.push("kanji-struggling");
          if (r.mined) classes.push("kanji-mined");
          if (r.kanji === picked) classes.push("kanji-picked");
          if (r.count >= solidAt) classes.push("kanji-solid");
          const rank = r.bccwj_rank ? `BCCWJ #${r.bccwj_rank}` : "not in BCCWJ";
          const title = `${r.kanji} · read ${r.count}× · ${rank}`;
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

      ${
        detail
          ? html`<${Inspector} row=${detail} />`
          : html`<${GridLegend} rule=${rule} />`
      }
    </div>
  `;
}

function GridLegend({ rule }) {
  // The red ring's rule, spelled out in the numbers it actually used — a ring
  // whose threshold you cannot see is a ring you cannot trust.
  const bar = (rule.baseline * rule.multiple * 100).toFixed(0);
  // "read" here means read *with the hooker running*. Lookups are only
  // recorded then, so pasted articles count toward how often a kanji has been
  // met and deliberately not toward what it cost. Saying so is the point: a
  // rate whose denominator differs from the count beside it has to admit it.
  const struggling = `looked up over ${bar}% as often as read — ${rule.multiple}× your average, over ${rule.floor}+ hooked readings`;
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
      <span class="legend-item legend-static">
        <span class="legend-swatch kanji-swatch kanji-mined"></span>on a card
      </span>
      <span class="legend-item legend-static">
        <span class="legend-swatch kanji-swatch kanji-struggling"></span
        >${struggling}
      </span>
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
  const rate =
    row.lookups > 0
      ? `${((row.lookups / Math.max(row.metered_count, 1)) * 100).toFixed(0)}%`
      : "";
  const cost = row.struggling
    ? `${looked} — ${rate} of readings, well past your average`
    : looked;
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
        <div class="kanji-detail-meta">${cost} · ${card}</div>
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
      ${
        other &&
        html`<p class="chart-note">
          Plus ${other.met.toLocaleString("en")} kanji no school grade teaches —
          jinmeiyō names and the hyōgai the writing reaches for. There is no
          denominator for those, which is why they get a count and not a bar.
        </p>`
      }
    </div>
  `;
}

/* The two cost lists ------------------------------------------------------ */

function HardestCard({ rows, rule }) {
  // Same encounter floor the red ring uses, so the top of this list is exactly
  // the ringed kanji rather than a second, differently-drawn answer.
  const hard = rows
    .filter((r) => r.metered_count >= rule.floor && r.lookups > 0)
    .sort((a, b) => b.lookups / b.metered_count - a.lookups / a.metered_count)
    .slice(0, HARD_LEN);
  if (!hard.length) {
    return html`
      <div class="card">
        <h2>What costs you most</h2>
        <p class="chart-empty">
          No lookups on kanji read often enough to rank.
        </p>
      </div>
    `;
  }
  const worst = hard[0].lookups / hard[0].metered_count;
  const pct = (rule.baseline * 100).toFixed(1);
  const bar = (rule.baseline * rule.multiple * 100).toFixed(0);
  const average = `Across everything you read the rate is ${pct}%; past ${bar}% the grid rings the kanji red.`;
  return html`
    <div class="card">
      <h2>What costs you most</h2>
      <div class="compare">
        ${hard.map((r) => {
          // Over hooked readings only — pasted text has no observable lookups.
          const rate = r.lookups / r.metered_count;
          const value = `${(rate * 100).toFixed(0)}%`;
          const unit = `${r.lookups} lookups / ${r.metered_count} hooked readings`;
          return html`
            <div class="compare-row">
              <span class="compare-name kanji-row-name"
                >${r.kanji}
                <span class="kanji-row-gloss">${r.gloss}</span></span
              >
              <span class="compare-track">
                <span
                  class="compare-fill"
                  style=${`width:${(rate / worst) * 100}%;background:var(--series-3)`}
                ></span>
              </span>
              <span class="compare-value" title=${unit}>${value}</span>
            </div>
          `;
        })}
      </div>
      <p class="chart-note">
        Lookups on words containing each kanji, per time the kanji was read.
        Compounds credit both halves, so this is not "which kanji I don't know"
        — it is which ones keep turning up in words that stop you. ${average}
      </p>
    </div>
  `;
}

function GapCard({ rows, solidAt }) {
  const gap = rows.filter((r) => r.count >= solidAt && !r.mined);
  if (!gap.length) return null;
  return html`
    <div class="card">
      <h2>Read often, never carded</h2>
      <p class="chart-note" style="margin:0 0 10px">
        ${gap.length.toLocaleString("en")} kanji read at least ${solidAt} times
        that no card in the deck contains. Familiar by exposure alone — some are
        already known, and the rest are the cheapest mining there is.
      </p>
      <div class="word-chips">
        ${gap
          .slice(0, GAP_LEN)
          .map(
            (r) =>
              html`<span class="chip"
                >${r.kanji} <b>×${r.count}</b>
                <span class="chip-note">${r.gloss}</span></span
              >`,
          )}
        ${gap.length > GAP_LEN && html`<span class="chip">…</span>`}
      </div>
    </div>
  `;
}

/* Per-work fingerprints --------------------------------------------------- */

function WorksCard({ works }) {
  if (!works.length) return null;
  return html`
    <div class="card">
      <h2>What each work is made of</h2>
      ${works.map(
        (w) => html`
          <div class="kanji-work">
            <div class="compare-title">
              ${w.work} · ${w.unique.toLocaleString("en")} distinct ·
              ${w.encounters.toLocaleString("en")} kanji
            </div>
            <div class="word-chips">
              ${w.distinctive.map(([k, n, ratio]) => {
                const note = `${ratio.toFixed(1)}× · ${n}`;
                return html`<span class="chip"
                  >${k} <span class="chip-note">${note}</span></span
                >`;
              })}
            </div>
          </div>
        `,
      )}
      <p class="chart-note">
        Not the commonest kanji in each work — the ones it leans on hardest
        compared with everything else you have read. 3.0× means three times its
        usual share of the page.
      </p>
    </div>
  `;
}
