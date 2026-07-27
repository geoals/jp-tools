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
import { DailyBarChart, ProgressBar } from "../charts.js";
import { api } from "../api.js";
import { fmtChars, fmtDateStr, fmtHours, fmtMins } from "../lib/format.js";
import { WorkMetaForm, setCurrentWork } from "../panels/work-form.js";

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

  useEffect(() => {
    setDetail(null);
    setError(null);
    api(`/api/works/detail?work=${encodeURIComponent(work)}`)
      .then(setDetail)
      .catch((e) => setError(e.message));
  }, [work]);

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
              dialogueByDate=${{}}
              metric="chars"
              split=${false}
              targetMins=${0}
            />`
          : html`<p class="chart-empty">No reading days recorded.</p>`
      }
      <p class="chart-note">
        This work's own reading days, in order — days it was not touched are not
        drawn.
      </p>
    </div>

    <${GaveCard}
      mined=${detail.mined}
      minedCount=${detail.mined_count}
      taught=${detail.taught}
      taughtCount=${detail.taught_count}
      days=${detail.days}
      firstRead=${detail.first_read}
    />
    <${VocabCard}
      vocab=${detail.vocabulary}
      unknown=${detail.top_unknown}
      distinctive=${detail.distinctive}
    />
    <${ProseCard}
      prose=${detail.prose}
      corpus=${detail.corpus_prose}
      workChars=${detail.chars}
    />
    <${SittingsCard} sittings=${detail.sittings} />
  `;
}

/** What came out of the work: cards mined while reading it, and the words it
 *  put in front of you first.
 *
 *  Both attribute by time — note ids are epoch milliseconds and the ledger
 *  holds each term's first sighting — so a card added while nothing was hooked
 *  belongs to no work rather than to a guessed one. */
function GaveCard({ mined, minedCount, taught, taughtCount, days, firstRead }) {
  if (!minedCount && !taughtCount) return null;
  const perDay = (days || []).filter((d) => d.cards > 0);
  const peak = Math.max(...perDay.map((d) => d.cards), 1);

  return html`
    <div class="card">
      <h2>What it gave you</h2>
      <div class="tile-row">
        <div class="tile">
          <div class="label">cards mined here</div>
          <div class="value">${minedCount.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">words met here first</div>
          <div class="value">${taughtCount.toLocaleString("en")}</div>
        </div>
      </div>

      ${
        perDay.length > 1 &&
        html`<div class="mine-strip">
          ${perDay.map((d) => {
            const label = `${d.date}: ${d.cards} card${d.cards === 1 ? "" : "s"}`;
            return html`<span
              class="mine-bar"
              title=${label}
              style=${`height:${Math.max(8, (d.cards / peak) * 44)}px`}
            ></span>`;
          })}
        </div>`
      }
      ${
        perDay.length > 1 &&
        html`<p class="chart-note">
          Cards added per reading day. The rate falling as a work goes on is the
          work getting easier.
        </p>`
      }

      <${TermList}
        title="Words it put in front of you first"
        note=${`Known now, and first met while reading this — as far back as the line stream goes. A work read before tracking began, or the earliest one in it, inherits words you already knew: this one starts at ${firstRead ?? "the beginning"}.`}
        terms=${taught}
      />
      ${
        mined.length > 0 &&
        html`<div class="term-list">
          <div class="word-list-label">Latest cards from it</div>
          <div class="word-chips">
            ${mined.map((m) => html`<span class="chip">${m.vocab}</span>`)}
          </div>
        </div>`
      }
    </div>
  `;
}

/** What this work is made of, in words, and how much of it you have.
 *
 *  Two coverage figures, never one. By type — of the distinct words here, how
 *  many are known — is how much studying it needs. By token — of the running
 *  text — is how it will feel to read. They sit forty points apart in practice,
 *  because the words a work repeats are the ones you already know. */
function VocabCard({ vocab, unknown, distinctive }) {
  if (!vocab || !vocab.types) {
    return html`<div class="card">
      <h2>What it is made of</h2>
      <p class="chart-empty">
        No vocabulary counted yet — the per-work ledger fills on the next Anki
        refresh.
      </p>
    </div>`;
  }
  const typePct = `${vocab.known_type_pct.toFixed(0)}%`;
  const tokenPct = `${vocab.known_token_pct.toFixed(0)}%`;
  const typeOf = ` (${vocab.known_types.toLocaleString("en")} / ${vocab.types.toLocaleString("en")})`;
  const tokenOf = ` (${vocab.known_tokens.toLocaleString("en")} / ${vocab.tokens.toLocaleString("en")})`;

  return html`
    <div class="card">
      <h2>What it is made of</h2>
      <div class="tile-row">
        <div class="tile">
          <div class="label">distinct words</div>
          <div class="value">${vocab.types.toLocaleString("en")}</div>
        </div>
        <div
          class="tile"
          title="Of the distinct words here, how many you know — how much studying it needs."
        >
          <div class="label">known, by word</div>
          <div class="value">
            ${typePct}<span class="value-sub">${typeOf}</span>
          </div>
        </div>
        <div
          class="tile"
          title="Of the running text, how much you know — how it will feel to read."
        >
          <div class="label">known, by text</div>
          <div class="value">
            ${tokenPct}<span class="value-sub">${tokenOf}</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">never judged</div>
          <div class="value">${vocab.new_types.toLocaleString("en")}</div>
        </div>
      </div>
      <p class="chart-note">
        Both figures, because they answer different questions and sit far apart:
        the words a work repeats are the ones you already know. Only words a
        dictionary recognizes are counted.
      </p>

      <${TermList}
        title="Unknown, commonest here first"
        note="Ranked by how often each occurs in this work, not by how common it
              is in general — that is what makes it a list for this work.
              Anything judged known or already carded is out; names drop out as
              you mark them in triage."
        terms=${unknown}
      />
      <${TermList}
        title="Words it keeps to itself"
        note="Used here and barely anywhere else in your reading. Learning these
              buys this work and little beyond it, which is worth knowing before
              spending a week on them."
        terms=${distinctive}
      />
    </div>
  `;
}

function TermList({ title, note, terms }) {
  if (!terms || !terms.length) return null;
  return html`
    <div class="term-list">
      <div class="word-list-label">${title}</div>
      <div class="word-chips">
        ${terms.map((t) => {
          const reading =
            t.reading && t.reading !== t.headword ? t.reading : "";
          const here = `×${t.count}`;
          const elsewhere =
            t.elsewhere > 0 ? `, ${t.elsewhere} elsewhere` : ", only here";
          return html`<span class="chip" title=${`${here}${elsewhere}`}>
            ${t.headword}
            ${reading && html`<span class="chip-reading">${reading}</span>`}
            <b>${here}</b>
          </span>`;
        })}
      </div>
      <p class="chart-note">${note}</p>
    </div>
  `;
}

/** How something compares with the rest of your reading, as "1.4× your usual".
 *  A bare percentage here says nothing: 38% dialogue is talky or sparse
 *  depending entirely on what the other 38 works do. */
function ratio(mine, theirs) {
  if (!mine || !theirs) return null;
  const r = mine / theirs;
  if (r >= 0.97 && r <= 1.03) return "about your usual";
  return `${r.toFixed(2)}× your usual`;
}

/** What the writing is like — context for the numbers above it, not a finding
 *  of its own. Kept to one row of facts on purpose. */
function ProseCard({ prose, corpus, workChars }) {
  if (!prose || !prose.chars) return null;
  const share = (prose.dialogue_chars / prose.chars) * 100;
  const corpusShare = corpus.chars
    ? (corpus.dialogue_chars / corpus.chars) * 100
    : null;
  const shareNote = corpusShare !== null ? ratio(share, corpusShare) : null;
  const lenNote = ratio(prose.median_len, corpus.median_len);
  // Built whole — htm collapses the whitespace where a literal and an
  // interpolation straddle a line break.
  const sentenceLabel = prose.median_len
    ? `${prose.median_len} chars`
    : `too few sentences yet (${prose.sentences.toLocaleString("en")})`;
  const tailLabel = prose.p90_len
    ? `longest tenth from ${prose.p90_len}`
    : null;
  const spoken = prose.dialogue_median_len;
  const narrated = prose.narration_median_len;
  const registerLabel =
    spoken && narrated ? `${spoken} spoken vs ${narrated} narrated` : null;
  // Only text the tracker actually captured can be measured. A work read
  // mostly before hooking — logged by hand with a character count and no text
  // — has prose figures drawn from whatever fraction was hooked, and saying
  // which fraction is the difference between a caveat and a wrong number.
  const covered = workChars ? prose.chars / workChars : 1;
  const coverageNote =
    covered < 0.9
      ? ` Over the ${(covered * 100).toFixed(0)}% of this work whose text was captured — the rest was logged by hand, as a character count with no text to read.`
      : "";

  return html` <div class="card">
    <h2>What the prose is like</h2>
    <div class="prose-facts">
      ${
        prose.brackets_dialogue
          ? html`<div class="prose-fact">
              <span class="label">dialogue</span>
              <span class="value">${`${share.toFixed(0)}%`}</span>
              ${shareNote && html`<span class="prose-note">${shareNote}</span>`}
            </div>`
          : html`<div class="prose-fact">
              <span class="label">dialogue</span>
              <span class="value">—</span>
              <span class="prose-note">this text does not bracket speech</span>
            </div>`
      }
      <div class="prose-fact">
        <span class="label">median sentence</span>
        <span class="value">${sentenceLabel}</span>
        ${lenNote && html`<span class="prose-note">${lenNote}</span>`}
      </div>
      ${
        tailLabel &&
        html`<div class="prose-fact">
          <span class="label">the tail</span>
          <span class="value">${tailLabel}</span>
        </div>`
      }
      ${
        registerLabel &&
        html`<div class="prose-fact">
          <span class="label">by register</span>
          <span class="value">${registerLabel}</span>
        </div>`
      }
    </div>
    <p class="chart-note">
      Measured against everything else you have read, because a share on its own
      says nothing. Sentence lengths exclude punctuation, like every other
      character count here; the register split takes only lines that are wholly
      one or the other.
    </p>
  </div>`;
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
