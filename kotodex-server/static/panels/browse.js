// Reading the ledger, as opposed to judging it.
//
// The triage passes each choose their own rows and ask for a verdict on every
// one. This answers the other question a bulk import raises: what is actually in
// here, and where did it come from.
//
// Ordered rarest-first by default because that is the axis an import goes wrong
// along. A threshold rule ("met ten times in one book") claims common words
// correctly and rare ones on the strength of a single book's subject matter, so
// the tail is where the mistakes are, and going down it is the cheapest audit
// there is. Rows with no BCCWJ rank sort *after* the ranked ones rather than
// first: the corpus is written text and its silence about この野郎 is a gap in
// the corpus, not a claim about the word.
//
// The rank shown is the *word's*, not the spelling's — see `word_rank_sql`.
//
// Judging writes straight through `POST /api/vocab/judge`, one word at a time
// and optimistically — the point of a list you are scanning is that marking
// something takes no more thought than noticing it.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

const SOURCES = [
  ["seed", "bulk import"],
  ["triage", "judged by hand"],
  ["any", "any source"],
];

const STATUSES = [
  ["known", "known"],
  ["unknown", "unknown"],
  ["new", "unjudged"],
  ["any", "any status"],
];

export function BrowseView({ onJudged }) {
  const [source, setSource] = useState("seed");
  const [status, setStatus] = useState("known");
  const [commonFirst, setCommonFirst] = useState(false);
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState(null);
  const [err, setErr] = useState(null);

  useEffect(() => {
    let live = true;
    const q = new URLSearchParams({
      source,
      status,
      offset: String(offset),
      common_first: String(commonFirst),
    });
    api(`/api/vocab/browse?${q}`)
      .then((p) => live && setPage(p))
      .catch((e) => live && setErr(e.message));
    return () => {
      live = false;
    };
  }, [source, status, commonFirst, offset]);

  // Filters change what a page *is*, so the offset cannot survive them.
  function refilter(setter) {
    return (v) => {
      setOffset(0);
      setter(v);
    };
  }

  async function judge(term, reading, next) {
    const before = page;
    setPage((p) =>
      p && {
        ...p,
        terms: p.terms.map((t) =>
          t.headword === term && t.reading === reading
            ? { ...t, status: next, source: "triage" }
            : t,
        ),
      },
    );
    try {
      await api("/api/vocab/judge", {
        method: "POST",
        body: { judgements: [{ headword: term, reading, status: next }] },
      });
      onJudged?.();
    } catch (e) {
      setPage(before);
      setErr(e.message);
    }
  }

  if (err) return html`<p class="chart-empty">Failed: ${err}</p>`;
  if (!page) return html`<p class="meta-hint">Loading…</p>`;

  const from = page.total ? offset + 1 : 0;
  const to = Math.min(offset + page.limit, page.total);
  const shown = `${from.toLocaleString("en")}–${to.toLocaleString("en")} of ${page.total.toLocaleString("en")}`;
  const orderLabel = commonFirst ? "most common first" : "rarest first";

  return html`
    <div class="card">
      <h2>
        Browse
      </h2>
      <div class="triage-actions">
        <${Picker} options=${SOURCES} value=${source} onChange=${refilter(setSource)} />
        <${Picker} options=${STATUSES} value=${status} onChange=${refilter(setStatus)} />
        <button class="ghost" onClick=${() => refilter(setCommonFirst)(!commonFirst)}>
          ${orderLabel}
        </button>
      </div>
      <p class="meta-hint">${shown}</p>
      ${page.terms.length === 0
        ? html`<p class="chart-empty">Nothing matches.</p>`
        : html`
            <table class="days-table">
              <thead>
                <tr>
                  <th>word</th>
                  <th class="num">BCCWJ</th>
                  <th class="num">met</th>
                  <th class="num">looked up</th>
                  <th>status</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                ${page.terms.map(
                  (t) => html`
                    <tr key=${`${t.headword}|${t.reading}`}>
                      <td>${wordLabel(t)}</td>
                      <td class="num">${t.rank === null ? "—" : t.rank.toLocaleString("en")}</td>
                      <td class="num">${t.encounter_count}</td>
                      <td class="num">${t.lookup_count || ""}</td>
                      <td>${t.status}</td>
                      <td>
                        ${t.status !== "unknown" &&
                        html`<button
                          class="ghost"
                          onClick=${() => judge(t.headword, t.reading, "unknown")}
                        >
                          don't know
                        </button>`}
                        ${t.status !== "known" &&
                        html`<button
                          class="ghost"
                          onClick=${() => judge(t.headword, t.reading, "known")}
                        >
                          known
                        </button>`}
                      </td>
                    </tr>
                  `,
                )}
              </tbody>
            </table>
          `}
      <div class="triage-actions">
        <button
          class="ghost"
          disabled=${offset === 0}
          onClick=${() => setOffset(Math.max(0, offset - page.limit))}
        >
          ← previous
        </button>
        <button
          class="ghost"
          disabled=${to >= page.total}
          onClick=${() => setOffset(offset + page.limit)}
        >
          next →
        </button>
      </div>
    </div>
  `;
}

/** The word, plus how many spellings it stands for. Built whole rather than
 *  interpolated beside literal text — htm collapses the whitespace at a line
 *  break (see CLAUDE.md). */
function wordLabel(t) {
  const word =
    t.reading && t.reading !== t.headword
      ? `${t.headword}【${t.reading}】`
      : t.headword;
  return t.forms > 1 ? `${word} +${t.forms - 1}` : word;
}

/** A row of radio-ish buttons. Small enough that a `<select>` would hide the
 *  options behind a click for no gain. */
function Picker({ options, value, onChange }) {
  return html`
    <span class="triage-actions">
      ${options.map(
        ([v, label]) => html`
          <button
            class=${v === value ? "pause-btn" : "ghost"}
            onClick=${() => onChange(v)}
          >
            ${label}
          </button>
        `,
      )}
    </span>
  `;
}
