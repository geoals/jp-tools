// The master dictionary's escape hatch.
//
// Sankoku is a general-purpose dictionary and carries no 冪等性, no 可用性, and
// no stem of either — so no decomposition rule reaches them, and swapping in
// JMdict would admit them along with every idiom and orthographic variant.
// The dictionary keeps answering the dictionary's question; the reader answers
// theirs, one term at a time.
//
// Mining is what surfaces a term here, and structurally it has to be: the
// triage queue and the frequency pass both gate on the vocabulary predicate,
// so a word the master does not list can never be offered by reading-based
// triage. A card is the reader claiming the word. Read-often terms join them
// because the tokenizer sometimes cannot decompose a compound the master would
// have listed in parts (懲罰房, met 61 times, credited to nothing).
//
// Promoting says "this is a word", not "I know this word" — the two questions
// have different answers for anything still in this list, so the button never
// touches `status`.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

export function PromotionView({ onPromoted }) {
  const [data, setData] = useState(null);
  const [picked, setPicked] = useState(() => new Set());
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);

  async function load() {
    setErr(null);
    try {
      const res = await api("/api/vocab/promotion-queue");
      setData(res);
      setPicked(new Set());
    } catch (e) {
      setErr(e.message);
    }
  }

  useEffect(() => {
    load();
  }, []);

  function toggle(key) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  async function submit() {
    if (!picked.size) return;
    setBusy(true);
    setErr(null);
    try {
      const terms = data.terms
        .filter((t) => picked.has(key(t)))
        .map((t) => ({ headword: t.headword, reading: t.reading }));
      await api("/api/vocab/promote", { method: "POST", body: { terms } });
      await load();
      onPromoted?.();
    } catch (e) {
      setErr(e.message);
    } finally {
      setBusy(false);
    }
  }

  if (err) return html`<div class="card"><p class="chart-empty">Failed: ${err}</p></div>`;
  if (!data) return html`<div class="card"><p class="chart-empty">Loading…</p></div>`;

  if (!data.terms.length) {
    return html`
      <div class="card">
        <p class="chart-empty">
          Nothing outside the master dictionary is waiting on a decision.
        </p>
      </div>
    `;
  }

  // Built whole: htm collapses whitespace at a line break, and prettier
  // reflows markup there freely.
  const heading = `${data.pending.toLocaleString("en")} terms the master dictionary does not list`;
  const chosen = `${picked.size} selected`;

  return html`
    <div class="card">
      <h2>count as vocabulary</h2>
      <p class="meta-hint">
        ${heading}. Mined ones are cards you made; the rest were read at least
        ${data.min_encounters} times. Promoting says it is a word, not that you
        know it — it never changes a status.
      </p>
      <div class="triage-actions">
        <span class="meta-hint">${chosen}</span>
        <button class="pause-btn" onClick=${submit} disabled=${busy || !picked.size}>
          ${busy ? "saving…" : "count these as vocabulary"}
        </button>
      </div>
      <table class="days">
        <thead>
          <tr>
            <th></th>
            <th>term</th>
            <th>reading</th>
            <th>source</th>
            <th>read</th>
          </tr>
        </thead>
        <tbody>
          ${data.terms.map((t) => {
            const k = key(t);
            return html`
              <tr key=${k}>
                <td>
                  <input
                    type="checkbox"
                    checked=${picked.has(k)}
                    onChange=${() => toggle(k)}
                  />
                </td>
                <td>${t.headword}</td>
                <td>${t.reading === t.headword ? "" : t.reading}</td>
                <td>${t.mined ? "mined" : "read"}</td>
                <td>${t.encounter_count.toLocaleString("en")}</td>
              </tr>
            `;
          })}
        </tbody>
      </table>
    </div>
  `;
}

const key = (t) => `${t.headword}|${t.reading}`;
