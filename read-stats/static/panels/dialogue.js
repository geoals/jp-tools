// Speech against prose: how much of the reading was people talking, and
// whether the two read at different speeds.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";
import { SegmentedControl } from "../components/controls.js";

const DIALOGUE_COLOR = "var(--series-1)";

const NARRATION_COLOR = "var(--series-2)";

function CompareBars({ title, unit, rows, format }) {
  const max = Math.max(...rows.map((r) => r.value));
  return html`
    <div class="compare">
      <div class="compare-title">${title}</div>
      ${rows.map(
        (r) => html`
          <div class="compare-row">
            <span class="compare-name">${r.label}</span>
            <span class="compare-track">
              <span
                class="compare-fill"
                style=${`width:${max > 0 ? (r.value / max) * 100 : 0}%;background:${r.color}`}
              ></span>
            </span>
            <span class="compare-value">${format(r.value)}</span>
          </div>
        `,
      )}
      <div class="compare-unit">${unit}</div>
    </div>
  `;
}

/** The comparison in words. Derived rather than written out, so it stays true
    if the two ever swap places — which is the interesting case, not a bug. */

function dialogueVerdict(d, n) {
  if (!d.speed || !n.speed) return null;
  const slower = n.speed < d.speed ? "narration" : "dialogue";
  const faster = slower === "narration" ? "dialogue" : "narration";
  const slowPct = Math.round(
    (1 - Math.min(d.speed, n.speed) / Math.max(d.speed, n.speed)) * 100,
  );
  const speedPart = `${slower} reads ${slowPct}% slower than ${faster}`;

  if (d.lookups_per_1k === null || n.lookups_per_1k === null) {
    return `${speedPart[0].toUpperCase()}${speedPart.slice(1)}.`;
  }
  const denser = n.lookups_per_1k > d.lookups_per_1k ? "narration" : "dialogue";
  const ratio = (
    Math.max(d.lookups_per_1k, n.lookups_per_1k) /
    Math.min(d.lookups_per_1k, n.lookups_per_1k)
  ).toFixed(1);
  const lookupPart = `takes ${ratio}× the lookups per character`;

  // The two measures usually agree — the slower half is the denser one — and
  // when they do the card can say which half is simply harder. When they part
  // company that is the finding, so don't paper over it with one clause.
  return denser === slower
    ? `${slower === "narration" ? "Prose" : "Speech"} is the harder half: ${speedPart} and ${lookupPart}.`
    : `${speedPart[0].toUpperCase()}${speedPart.slice(1)}, yet ${denser} ${lookupPart} — the slower half is not the one with more unknown words.`;
}

export function DialogueCard({ dialogue, currentWork }) {
  // "all" = every VN pooled (the passed-in lifetime data); "work" = just the
  // current VN, fetched on demand so the lifetime view costs nothing.
  const [scope, setScope] = useState("all");
  const [workData, setWorkData] = useState(null);
  const [workErr, setWorkErr] = useState(false);

  useEffect(() => {
    if (scope !== "work" || !currentWork) return;
    let live = true;
    setWorkData(null);
    setWorkErr(false);
    api(`/api/dialogue/summary?days=60&work=${encodeURIComponent(currentWork)}`)
      .then((r) => live && setWorkData(r))
      .catch(() => live && setWorkErr(true));
    return () => {
      live = false;
    };
  }, [scope, currentWork]);

  // The toggle only appears once there's a current work to scope to; without
  // one the card stays lifetime-only, as it was before.
  const head = html`
    <div class="card-head">
      <h2>Dialogue vs narration</h2>
      ${
        currentWork &&
        html`<${SegmentedControl}
          label="Scope"
          value=${scope}
          onChange=${setScope}
          options=${[
            { value: "all", label: "all works" },
            { value: "work", label: "current work" },
          ]}
        />`
      }
    </div>
  `;

  const data = scope === "work" ? workData : dialogue;
  const scopeNote =
    scope === "work"
      ? `Just ${currentWork} — its own split, speed and lookup rate.`
      : "All works, all of your reading history, pooled together.";

  if (scope === "work" && workErr) {
    return html`<div class="card">
      ${head}
      <p class="chart-empty">Couldn't load this work's split.</p>
    </div>`;
  }
  if (!data) {
    return html`<div class="card">
      ${head}
      <p class="chart-empty">Loading…</p>
    </div>`;
  }
  if (data.overall.share === null) {
    return html`<div class="card">
      ${head}
      <p class="chart-empty">
        ${
          scope === "work"
            ? "No 「」-classified text for this work yet."
            : "Nothing classified yet — this reads 「」 out of the stored line text, so it fills in as the logger captures lines."
        }
      </p>
    </div>`;
  }

  const { dialogue: d, narration: n, share } = data.overall;
  const dPct = Math.round(share * 100);
  const today = data.today;

  // Pre-built strings: htm collapses whitespace where literal text meets an
  // ${...} across a line break (see CLAUDE.md).
  const dLabel = `dialogue ${dPct}%`;
  const nLabel = `narration ${100 - dPct}%`;
  const todayLabel =
    today.share === null
      ? "nothing hooked today yet — split on 「」 in the line text"
      : `today ${Math.round(today.share * 100)}% dialogue · split on 「」 in the line text`;

  const verdict = dialogueVerdict(d, n);

  return html`
    <div class="card">
      ${head}
      <div class="meta-hint">${scopeNote}</div>

      <div class="split-bar">
        <span
          class="split-seg"
          style=${`width:${dPct}%;background:${DIALOGUE_COLOR}`}
        ></span>
        <span
          class="split-seg"
          style=${`width:${100 - dPct}%;background:${NARRATION_COLOR}`}
        ></span>
      </div>
      <div class="split-caption">
        <span
          ><span
            class="legend-swatch"
            style=${`background:${DIALOGUE_COLOR}`}
          ></span
          >${dLabel}</span
        >
        <span
          ><span
            class="legend-swatch"
            style=${`background:${NARRATION_COLOR}`}
          ></span
          >${nLabel}</span
        >
      </div>
      <div class="meta-hint">${todayLabel}</div>

      <${CompareBars}
        title="reading speed"
        unit="chars/hour, over lines that were wholly one or the other"
        format=${(v) => Math.round(v).toLocaleString("en")}
        rows=${[
          { label: "dialogue", value: d.speed, color: DIALOGUE_COLOR },
          { label: "narration", value: n.speed, color: NARRATION_COLOR },
        ]}
      />

      ${
        d.lookups_per_1k !== null &&
        n.lookups_per_1k !== null &&
        html`
          <${CompareBars}
            title="unknown-word rate"
            unit="lookups per 1000 characters"
            format=${(v) => v.toFixed(2)}
            rows=${[
              {
                label: "dialogue",
                value: d.lookups_per_1k,
                color: DIALOGUE_COLOR,
              },
              {
                label: "narration",
                value: n.lookups_per_1k,
                color: NARRATION_COLOR,
              },
            ]}
          />
        `
      }
      ${verdict && html`<p class="chart-note">${verdict}</p>`}
    </div>
  `;
}
