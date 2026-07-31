// `#tokenize` — paste a line, see what the pipeline made of it.
//
// A page of its own rather than a tab, for the same reason `#read` is one: it
// needs none of the dashboard's poll, and it is opened to answer a question
// about the tokenizer, not about the reading.
//
// It shows the same answer twice, because they are two different questions.
// The text at the top is tinted exactly as the feed tints it — the check that
// the marks land on the words you expected, which is a thing you can only see
// in place. The table under it is the pipeline's own output, one row per token
// including the ones the feed drops, since "why was this word never asked
// about" is usually answered by a `name` or a `non-word` in that column.
//
// Marks here are inline `<span>`s, not the feed's positioned layer. The layer
// exists so Yomitan sees one text node per line; nothing scans this page, so
// the plain way is the right one here.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";

/** The tiers the feed paints. `known` is returned and deliberately not tinted —
 *  the absence of a mark is what makes the marks readable — so it is left plain
 *  here too, and named in the table instead. */
const PAINTED = ["new", "seen", "unknown"];

const SAMPLE = "彼女はしゃくりあげながら、東京の懲罰房のことを話した。";

export function TokenizeView() {
  const [text, setText] = useState("");
  const [result, setResult] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);

  async function submit(e) {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await api("/api/tokenize", { method: "POST", body: { text } }));
    } catch (err) {
      setError(err.message);
      setResult(null);
    } finally {
      setBusy(false);
    }
  }

  return html`
    <div class="tokenize-page">
      <header class="tokenize-header">
        <h1>tokenize</h1>
        <a class="pause-btn" href="#today">← dashboard</a>
      </header>

      <form class="card tokenize-form" onSubmit=${submit}>
        <textarea
          rows="3"
          placeholder="Paste Japanese text"
          value=${text}
          onInput=${(e) => setText(e.target.value)}
        ></textarea>
        <div class="tokenize-actions">
          <button type="submit" disabled=${busy || !text.trim()}>
            ${busy ? "tokenizing…" : "tokenize"}
          </button>
          <button
            type="button"
            class="linkish"
            onClick=${() => setText(SAMPLE)}
          >
            use a sample line
          </button>
        </div>
      </form>

      ${error && html`<div class="card tokenize-error">${error}</div>`}
      ${result && html`<${Result} result=${result} />`}
    </div>
  `;
}

function Result({ result }) {
  const words = result.tokens.filter((t) => t.status);
  const counted = `${result.tokens.length} tokens · ${words.length} words`;
  return html`
    <div class="card">
      <p class="tokenize-text">${segments(result.text, result.tokens)}</p>
      <${Legend} />
    </div>
    <div class="card">
      <div class="card-head">
        <h2>tokens</h2>
        <span class="muted">${counted}</span>
      </div>
      <table class="tokenize-table">
        <thead>
          <tr>
            <th>surface</th>
            <th>headword</th>
            <th>reading</th>
            <th>part of speech</th>
            <th>status</th>
            <th class="num">met</th>
            <th class="num">looked up</th>
          </tr>
        </thead>
        <tbody>
          ${result.tokens.map(
            (t, i) => html`
              <tr key=${`${t.start}-${i}`} class=${t.status ? "" : "excluded"}>
                <td class="jp">${t.surface}</td>
                <td class="jp">${t.headword}</td>
                <td class="jp">${t.reading}</td>
                <td class="muted">${t.pos}</td>
                <td>
                  <span
                    class=${`tok-status ${t.status || "dropped"}`}
                    title=${
                      t.status
                        ? "the tier the reading view would tint it"
                        : "no mark in the reading view"
                    }
                    >${t.status || t.excluded}</span
                  >
                </td>
                <td class="num">${count(t.encounter_count)}</td>
                <td class="num">${count(t.lookup_count)}</td>
              </tr>
            `,
          )}
        </tbody>
      </table>
    </div>
  `;
}

/** An em dash, not a zero: no ledger row at all is a different fact from a row
 *  that has never been met. */
function count(n) {
  return n === null || n === undefined ? "—" : n.toLocaleString("en");
}

function Legend() {
  const items = [
    ["new", "never judged, barely met"],
    ["seen", "never judged, met before"],
    ["unknown", "judged, and not known"],
    ["known", "known or mined — never tinted"],
    ["dropped", "no mark: grammar, name, non-word, blacklisted"],
  ];
  return html`
    <div class="tokenize-legend">
      ${items.map(
        ([tier, what]) => html`
          <span key=${tier} class="tokenize-legend-item">
            <span class=${`tok-status ${tier}`}>${tier}</span>
            <span class="muted">${what}</span>
          </span>
        `,
      )}
    </div>
  `;
}

/** The text, cut into tinted and untinted pieces.
 *
 *  Offsets are UTF-16 code units from the server, which is exactly what a
 *  JavaScript string is indexed in, so `slice` takes them unchanged. Tokens
 *  arrive in reading order and never overlap; anything the cursor has already
 *  passed is skipped rather than trusted, so a bad offset costs one mark
 *  instead of scrambling the line. */
function segments(text, tokens) {
  const out = [];
  let at = 0;
  for (const t of tokens) {
    if (
      !PAINTED.includes(t.status) ||
      t.start < at ||
      t.start + t.len > text.length
    )
      continue;
    if (t.start > at) out.push(text.slice(at, t.start));
    const word = text.slice(t.start, t.start + t.len);
    out.push(html`<span class=${`tok-mark ${t.status}`}>${word}</span>`);
    at = t.start + t.len;
  }
  out.push(text.slice(at));
  return out;
}
