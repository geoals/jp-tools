// One work's own page: how it was read, sitting by sitting.
//
// Fetched on its own (`/api/works/detail?work=…`) rather than sliced from the
// dashboard poll — it is the whole line stream filtered to one title, which is
// too much to send for every work on the shelf just in case one is opened.
//
// The bars are the work's *own* reading days, not a calendar window. A VN read
// in four sittings over two weeks gets four bars, not fourteen with ten empty:
// what the shape is meant to show is how the reading was distributed, and a
// month of zeroes around it shows nothing.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { DailyBarChart, ProgressBar, SpeedTrendChart } from "../charts.js";
import { api } from "../api.js";
import { fmtChars, fmtDateStr, fmtHours, fmtMins } from "../lib/format.js";
import { WorkMetaForm, setCurrentWork } from "../panels/work-form.js";
import { WorkTriage } from "../panels/work-triage.js";

const SITTINGS_SHOWN = 20;

/** The synthetic work every logged article aggregates under
 *  (`stats::work::ARTICLES_WORK`). Nothing hooks an article, so it can never
 *  be what the logger stamps lines with. */
const ARTICLES = "Articles";

export function WorkDetail({ work, works, settings, onBack, onSaved }) {
  const [detail, setDetail] = useState(null);
  const [error, setError] = useState(null);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [triaging, setTriaging] = useState(false);

  const isCurrent = settings?.current_work === work;
  // "Read this next" points the logger at a title: every line it captures from
  // here on is stamped with it. Meaningless for Articles, which is not a
  // hookable thing but a bucket for text logged after the fact.
  const canMakeCurrent = !isCurrent && work !== ARTICLES;

  async function makeCurrent() {
    setBusy(true);
    try {
      await setCurrentWork(work);
      onSaved();
    } catch (e) {
      alert(e.message);
    } finally {
      setBusy(false);
    }
  }

  // Named rather than inline so leaving a triage session can re-run it: the
  // session exists to move the figures this fetches.
  const load = () => {
    setDetail(null);
    setError(null);
    api(`/api/works/detail?work=${encodeURIComponent(work)}`)
      .then(setDetail)
      .catch((e) => setError(e.message));
  };
  useEffect(load, [work]);

  // The shelf row for this work, which is what `WorkMetaForm` edits by id.
  const row = works.find((w) => w.work === work) ?? null;

  const back = html`
    <button class="ghost" onClick=${onBack}>← library</button>
  `;

  if (error) {
    return html`<div class="card">
      <div class="card-head">
        <h2>${work}</h2>
        ${back}
      </div>
      <p class="chart-empty">${error}</p>
    </div>`;
  }
  if (!detail) {
    return html`<div class="card">
      <div class="card-head">
        <h2>${work}</h2>
        ${back}
      </div>
      <p class="chart-empty">Loading…</p>
    </div>`;
  }

  const meta = detail.meta;
  const total = meta?.total_chars;
  const pct = total ? Math.min(100, (detail.chars / total) * 100) : null;
  const speed = detail.speed;
  // Built whole: htm collapses whitespace where a literal meets an
  // interpolation across a line break.
  const readBetween = [
    fmtDateStr(detail.first_read),
    fmtDateStr(detail.last_read),
  ]
    .filter(Boolean)
    .join(" – ");
  const progressLabel =
    pct !== null
      ? `${fmtChars(detail.chars)} / ${fmtChars(total)} · ${pct.toFixed(0)}%`
      : null;
  const leftLabel =
    detail.remaining_secs !== null && detail.remaining_secs !== undefined
      ? `${fmtHours(detail.remaining_secs)} left at this work's ${fmtChars(Math.round(speed))}/h`
      : null;

  if (triaging) {
    // Reloads the page's own figures on the way out: a session's whole point
    // is to move them.
    return html`<${WorkTriage}
      work=${work}
      onBack=${() => {
        setTriaging(false);
        load();
      }}
    />`;
  }

  return html`
    <div class="card">
      <div class="card-head">
        <h2>
          ${detail.work}
          ${isCurrent && html`<span class="status-tag">current</span>`}
        </h2>
        <div class="card-controls">
          ${
            canMakeCurrent &&
            html`<button class="ghost" disabled=${busy} onClick=${makeCurrent}>
              ${busy ? "…" : "read this"}
            </button>`
          }
          ${
            meta &&
            html`<button class="ghost" onClick=${() => setEditing((v) => !v)}>
              ${editing ? "close" : "edit"}
            </button>`
          }
          ${back}
        </div>
      </div>

      <div class="work-detail-head">
        ${meta?.cover && html`<img class="cover" src=${meta.cover} alt="" />`}
        <div class="work-detail-facts">
          <div class="tile-row">
            <div class="tile">
              <div class="label">characters</div>
              <div class="value">${detail.chars.toLocaleString("en")}</div>
            </div>
            <div class="tile">
              <div class="label">time</div>
              <div class="value">${fmtHours(detail.active_secs)}</div>
            </div>
            <div class="tile">
              <div class="label">speed</div>
              <div class="value">
                ${speed ? `${fmtChars(Math.round(speed))}/h` : "—"}
              </div>
            </div>
            <div class="tile">
              <div class="label">sittings</div>
              <div class="value">${detail.sittings.length}</div>
            </div>
          </div>
          ${readBetween && html`<div class="meta-hint">read ${readBetween}</div>`}
          ${
            pct !== null &&
            html`<div class="work-detail-progress">
              <${ProgressBar}
                pct=${pct}
                label=${`Progress through ${detail.work}`}
              />
              <div class="progress-caption">
                <span>${progressLabel}</span>
                <span>${leftLabel ?? ""}</span>
              </div>
            </div>`
          }
        </div>
      </div>

      ${
        editing &&
        row &&
        html`<${WorkMetaForm}
          work=${row}
          onSaved=${() => {
            setEditing(false);
            onSaved();
          }}
          onCancel=${() => setEditing(false)}
        />`
      }
    </div>

    <div class="card">
      <h2>How it was read</h2>
      ${
        detail.days.length
          ? html`<${DailyBarChart}
              days=${detail.days}
              metric="chars"
              targetMins=${0}
            />`
          : html`<p class="chart-empty">No reading days recorded.</p>`
      }
    </div>

    <div class="card">
      <h2>Speed, day by day</h2>
      <${SpeedTrendChart} days=${detail.days} />
    </div>

    <${VocabCard}
      vocab=${detail.vocabulary}
      script=${detail.script}
      onTriage=${() => setTriaging(true)}
    />
    <${SittingsCard} sittings=${detail.sittings} />
  `;
}

/** The work's vocabulary, twice over: what has been met in it, and what its
 *  whole script holds.
 *
 *  Met-so-far alone is a sample of the work drawn by how far you happen to
 *  have read, and it flatters it — the words met first are the ones it
 *  repeats. The pair is the figure. By text says how the prose will read; by
 *  word says how much of its vocabulary is still ahead, and the two routinely
 *  disagree.
 *
 *  The script row needs an imported script (`jp-script profile`) and most
 *  works will never have one, so the card degrades to the met row alone. */
function VocabCard({ vocab, script, onTriage }) {
  if (!vocab || !vocab.types) return null;
  const rows = [{ label: "met so far", ...vocab }];
  if (script) rows.push({ label: "whole script", ...script });

  // Progress through the work's vocabulary, which is not progress through its
  // text: the long tail arrives late, so this trails the character count.
  const metPct =
    script && script.types
      ? Math.round((script.met_types / script.types) * 100)
      : null;
  const metLabel =
    metPct === null
      ? null
      : `${script.met_types.toLocaleString("en")} of ${script.types.toLocaleString("en")} words met — ${metPct}%`;

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Vocabulary</h2>
        ${
          script &&
          html`<div class="card-controls">
            <button class="ghost" onClick=${onTriage}>triage the script</button>
          </div>`
        }
      </div>
      <table class="vocab-split">
        <thead>
          <tr>
            <th></th>
            <th>distinct</th>
            <th title="Of the distinct words, how many you know — how much studying it needs.">
              known, by word
            </th>
            <th title="Of the running text, how much you know — how it will feel to read.">
              known, by text
            </th>
          </tr>
        </thead>
        <tbody>
          ${rows.map(
            (r) => html`<tr>
              <th scope="row">${r.label}</th>
              <td>${r.types.toLocaleString("en")}</td>
              <td>${Math.round(r.known_type_pct)}%</td>
              <td>${Math.round(r.known_token_pct)}%</td>
            </tr>`,
          )}
        </tbody>
      </table>
      ${
        metLabel &&
        html`<div class="progress-caption">
          <span>${metLabel}</span>
          <span title="A branching work's script holds every route, so more of it exists than any one playthrough reads.">
            whole script, every route
          </span>
        </div>`
      }
    </div>
  `;
}

/** Every sitting with the work, newest first: how long, how much, how fast.
 *
 *  Pace per sitting is the number worth watching — a VN whose vocabulary
 *  settles reads faster in its second half, and that is visible here and
 *  nowhere else on the dashboard. */
function SittingsCard({ sittings }) {
  const [all, setAll] = useState(false);
  if (!sittings.length) return null;
  const shown = all ? sittings : sittings.slice(0, SITTINGS_SHOWN);
  const more = sittings.length - shown.length;

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Sittings</h2>
        ${
          more > 0 &&
          html`<button class="ghost" onClick=${() => setAll(true)}>
            show all ${sittings.length}
          </button>`
        }
      </div>
      <table class="days">
        <thead>
          <tr>
            <th>date</th>
            <th>started</th>
            <th>time</th>
            <th>chars</th>
            <th>speed</th>
            <th>cards</th>
          </tr>
        </thead>
        <tbody>
          ${shown.map((s) => {
            const started = new Date(s.start_ts * 1000).toLocaleTimeString(
              "en-GB",
              { hour: "2-digit", minute: "2-digit" },
            );
            // Below ten minutes the denominator is noise, and an estimated
            // duration came *from* the pace — it can only report it back.
            const speed =
              !s.estimated && s.active_secs >= 600
                ? `${fmtChars(Math.round(s.chars / (s.active_secs / 3600)))}/h`
                : "—";
            const time = s.active_secs > 0 ? fmtMins(s.active_secs) : "—";
            return html`
              <tr>
                <td class="work-name">${s.date}</td>
                <td>${started}</td>
                <td>
                  ${time}${s.estimated && html`<span class="status-tag">est</span>`}
                </td>
                <td>${s.chars.toLocaleString("en")}</td>
                <td>${speed}</td>
                <td>${s.cards || "—"}</td>
              </tr>
            `;
          })}
        </tbody>
      </table>
    </div>
  `;
}
