// `#tokenize` — paste a line, see what the pipeline made of it.
//
// Reached from ⚙ rather than from a tab: it asks about the tokenizer, not about
// the reading. Renders inside the dashboard shell, so the header stays put.
//
// It shows the same answer twice, because they are two different questions.
// The text at the top is tinted exactly as the feed tints it — the check that
// the marks land on the words you expected, which is a thing you can only see
// in place. The table under it is the pipeline's own output, one row per token
// including the ones the feed drops, since "why was this word never asked
// about" is usually answered by a `name` or a `non-word` in that column.
//
// Each row also expands into how that word was written everywhere *else* it has
// been met, with a line for each spelling. The table above says what the
// pipeline made of the text in front of you; that says whether the same term
// has been arriving the same way all along — which is the other half of "did
// the tokenizer find a word or invent one".
//
// Marks here are inline `<span>`s, not the feed's positioned layer. The layer
// exists so Yomitan sees one text node per line; nothing scans this page, so
// the plain way is the right one here.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";
import { Spellings, useSpellings } from "../lib/spellings.js";

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
      <form class="card tokenize-form" onSubmit=${submit}>
        <h2>tokenize</h2>
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
      ${result && html`<${Trace} steps=${result.steps || []} />`}
    </div>
  `;
}

function Result({ result }) {
  const spellings = useSpellings();
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
            <th>judged as</th>
            <th>part of speech</th>
            <th>status</th>
            <th class="num">met</th>
            <th class="num">looked up</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          ${result.tokens.map(
            (t, i) => html`
              <tr key=${`${t.start}-${i}`} class=${t.status ? "" : "excluded"}>
                <td class="jp">${t.surface}</td>
                <td class="jp">${t.headword}</td>
                <td class="jp">${t.reading}</td>
                <td class="jp judged-as">${judgedAs(t)}</td>
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
                <td>
                  ${
                    t.headword
                      ? html`<button
                          class="ghost"
                          title="How this word was written everywhere else it has been met, and a line it appeared in"
                          onClick=${() => spellings.toggle(t)}
                        >
                          ${spellings.isOpen(t) ? "hide" : "how written"}
                        </button>`
                      : null
                  }
                </td>
              </tr>
              ${
                spellings.isOpen(t) &&
                html`<tr key=${`${t.start}-${i}-spellings`}>
                  <td colspan="9">
                    <${Spellings} data=${spellings.data(t)} />
                  </td>
                </tr>`
              }
            `,
          )}
        </tbody>
      </table>
    </div>
  `;
}

/** Whether a step could have gone the other way.
 *
 *  Most of a trace is the pipeline agreeing with itself — every particle gets a
 *  gate that keeps it and an identity with one candidate, and recomposition
 *  offers every run of two and three adjacent tokens at every position, nearly
 *  all of which spell nothing. Those are true and they are not why anything
 *  happened, and on a full line they bury the handful of steps that are.
 *
 *  So the default view is the decisions with a fork in them:
 *
 *  - a **rewrite** or a **stammer drop** — the line is not what was pasted.
 *  - a **join**, taken or refused. Never a run that named no word at all.
 *  - a **split**, which is the gate having refused a compound. The gate itself
 *    is left out: when it keeps a word there is nothing to explain, and when it
 *    doesn't, the split underneath says so and says what came out.
 *  - an **identity** with more than one candidate, or one that fell past the
 *    plain "the master lists this pair" rung. Choosing 奇麗 over 綺麗 is a fork;
 *    filing を under を is not.
 *
 *  The constant below has to stay the exact string `identity_ladder` returns
 *  for that rung — it is a filter keyed on prose, and rewording one end alone
 *  silently floods the default view. */
const ROUTINE_IDENTITY =
  "Exact match: master dictionary lists both spelling and reading";

/** Kana or kanji — anything that could be a word. A comma also gets an identity
 *  and always falls all the way down the ladder, which reads like a fork and is
 *  not one. */
const WORDLIKE = /[぀-ヿ一-龯]/;

function decisive(s) {
  switch (s.kind) {
    case "rewrite":
    case "stutter":
    // Always shown: it is where the boundaries came from, and every later step
    // only regroups them. Without it the trace reads as if the pipeline chose
    // to start at the first gate's surface.
    case "segment":
      return true;
    case "join":
      return s.verdict !== "no_signal";
    case "split":
      return s.parts.length > 1;
    case "identity":
      return (
        WORDLIKE.test(s.surface) &&
        (s.candidates.length > 1 || s.rule !== ROUTINE_IDENTITY)
      );
    default:
      return false;
  }
}

/** Every decision the pipeline made, in the order it made them.
 *
 *  The table above is the verdict; this is the reasoning, so a parse that looks
 *  wrong can be checked against the rule that produced it instead of guessed
 *  at. The server sends the tokenizer's own trace — the same run, recorded, not
 *  a reconstruction — so a step here is a line of the pipeline and not an
 *  opinion about one. */
function Trace({ steps }) {
  const [showAll, setShowAll] = useState(false);
  if (!steps.length) return null;
  const shown = showAll ? steps : steps.filter(decisive);
  const count = `${shown.length} of ${steps.length} decisions`;
  const toggle = showAll
    ? "Show only decisions with alternatives"
    : `Show all ${steps.length} steps, including routine matches`;
  return html`
    <div class="card">
      <div class="card-head">
        <h2>why</h2>
        <span class="muted">${count}</span>
      </div>
      <ol class="trace">
        ${shown.map((s, i) => html`<${TraceStep} key=${i} step=${s} />`)}
      </ol>
      <button
        type="button"
        class="linkish trace-toggle"
        onClick=${() => setShowAll(!showAll)}
      >
        ${toggle}
      </button>
    </div>
  `;
}

/** One step: the phase it belongs to, what went in and out, and the rule.
 *
 *  The badge names the *phase* and the message leads with the classification —
 *  the badge is not a second copy of the verdict, or every join row would read
 *  "refused / Blocked from merging". The taken/refused distinction is carried
 *  by the coloured edge and by the first word of the message.
 *
 *  Each branch builds its own text in JS rather than interleaving literals with
 *  `${...}` across lines — htm collapses the whitespace between them, and the
 *  arrows and slashes here are exactly the places that would silently close up. */
function TraceStep({ step }) {
  const s = step;
  let stage = s.kind;
  let main = "";
  let note = "";
  let tone = "";

  if (s.kind === "rewrite") {
    main = `${s.from} → ${s.to}`;
    note = "Emphatic っ removed before analysis";
  } else if (s.kind === "segment") {
    main = s.parts.join(" + ");
    note = `Sudachi's own segmentation at mode ${s.mode}. A boundary here is one its dictionary made, not a rule below.`;
  } else if (s.kind === "gate") {
    main = s.surface;
    note = s.why;
    tone = s.kept ? "kept" : "";
  } else if (s.kind === "split") {
    main =
      s.mode === "none" ? s.surface : `${s.surface} → ${s.parts.join(" + ")}`;
    note =
      s.mode === "none"
        ? "No further split available"
        : `Split at mode ${s.mode}`;
  } else if (s.kind === "stutter") {
    main = `${s.fragment} 、 ${s.into}`;
    note = `Stammer dropped: ${s.fragment} repeats the start of ${s.into}`;
  } else if (s.kind === "identity") {
    main = `${s.surface} → ${s.headword} / ${s.reading}`;
    note = s.rule;
  } else if (s.kind === "join") {
    main = s.parts.join(" + ");
    note = s.reason;
    if (s.verdict === "joined") {
      main = `${s.parts.join(" + ")} → ${s.term} / ${s.reading}`;
      note = s.signal;
      tone = "kept";
    } else if (s.verdict === "refused") {
      main = `${s.parts.join(" + ")} ↛ ${s.term}`;
      tone = "refused";
    }
  }

  const candidates =
    s.kind === "identity" && s.candidates.length
      ? `Candidates tried, best first: ${s.candidates.join("  ·  ")}`
      : "";

  return html`
    <li class=${`trace-step ${tone}`}>
      <span class="trace-stage">${stage}</span>
      <span class="trace-body">
        <span class="jp">${main}</span>
        ${note && html`<span class="trace-why">${note}</span>`}
        ${candidates && html`<span class="trace-cands">${candidates}</span>`}
      </span>
    </li>
  `;
}

/** The reading the *status* came from, when it is not this token's own.
 *
 *  The reading column is the tokenizer's output and nothing else — the suffix
 *  鬼 of 殺人鬼 is read き. But the status beside it can come from another row
 *  for the same headword (鬼/おに, marked known), because a word judged under one
 *  reading is not asked about again. That is worth seeing rather than hiding:
 *  the two readings are separate dictionary entries, and this column is where
 *  the folding shows up. */
function judgedAs(t) {
  return t.judged_as ? `via ${t.judged_as}` : "";
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
