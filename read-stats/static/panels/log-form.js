// Logging reading a texthooker cannot see: physical books, articles, ebooks.
//
// Two modes over one form and one POST. *pages* estimates the character count
// from a page count; *text* takes the thing that was read and counts it
// exactly. The estimate exists because a paper book has no text to paste — for
// anything on a screen the paste is strictly better, and it is what a later
// import will tokenize into the knowledge layer.
//
// Minutes are optional throughout. A book or an article is read without a
// stopwatch, and requiring the number only produces made-up ones; left blank,
// the duration is derived from the characters at the reader's own recent
// effective pace (`History::duration_of`).
//
// The count under the textarea comes from the server (`/api/text/count`)
// rather than from a `length` here: which characters count is a rule this
// codebase keeps in exactly one place, and a preview that disagreed with the
// stored number would be worse than no preview.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

const COUNT_DEBOUNCE_MS = 300;

// Today in the browser's own timezone. `toISOString` would be UTC and shows
// yesterday's date all evening at UTC+2. The server treats a date of today as
// "now" rather than mid-day, so the pre-fill costs nothing in accuracy.
function todayLocal() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function LogForm({ onLogged }) {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);
  const [mode, setMode] = useState("pages");
  const [content, setContent] = useState("");
  const [minutes, setMinutes] = useState("");
  // Controlled, not a bare default: Preact rewrites an uncontrolled input's
  // value on every re-render, so typing in any other field would snap the date
  // back to today.
  const [date, setDate] = useState(todayLocal());
  const [counted, setCounted] = useState(null);
  // Follows the mode — pasting text is nearly always an article — but stays
  // overridable, since an ebook chapter pastes the same way.
  const [source, setSource] = useState("book");

  const text = content.trim();

  useEffect(() => {
    if (!text) {
      setCounted(null);
      return;
    }
    let live = true;
    const t = setTimeout(async () => {
      try {
        const r = await api("/api/text/count", {
          method: "POST",
          body: { content: text },
        });
        if (live) setCounted(r.chars);
      } catch {
        if (live) setCounted(null);
      }
    }, COUNT_DEBOUNCE_MS);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [text]);

  // Same shape as the speed figures elsewhere: characters, and what they come
  // to per hour over the minutes entered beside them.
  let preview = null;
  if (counted !== null) {
    const chars = counted.toLocaleString();
    const mins = Number(minutes);
    preview =
      mins > 0
        ? `${chars} chars · ${Math.round((counted / mins) * 60).toLocaleString()} chars/h`
        : `${chars} chars · time estimated from your recent pace`;
  }

  async function submit(e) {
    e.preventDefault();
    const f = e.currentTarget;
    const body = {
      date: f.date.value || undefined,
      minutes: f.minutes.value ? Number(f.minutes.value) : undefined,
      work: f.work.value || undefined,
      source: f.source.value,
    };
    if (mode === "text") {
      body.content = text || undefined;
      body.url = f.url.value || undefined;
    } else {
      body.pages = f.pages.value ? Number(f.pages.value) : undefined;
      body.chars = f.chars.value ? Number(f.chars.value) : undefined;
    }
    setBusy(true);
    setMsg(null);
    try {
      const s = await api("/api/sessions", { method: "POST", body });
      setMsg({ ok: true, text: `logged ${s.chars.toLocaleString()} chars ✓` });
      setMinutes("");
      setContent("");
      f.minutes.value = "";
      if (mode === "text") {
        f.url.value = "";
      } else {
        f.pages.value = "";
        f.chars.value = "";
      }
      onLogged();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  const modeButton = (value, label) => html`
    <button
      type="button"
      class="segment ${mode === value ? "segment-on" : ""}"
      onClick=${() => {
        setMode(value);
        setSource(value === "text" ? "article" : "book");
      }}
    >
      ${label}
    </button>
  `;

  return html`
    <div class="card">
      <details class="log">
        <summary>
          Log reading (physical book, article, anything unhooked)
        </summary>
        <div class="log-modes">
          <div class="segmented">
            ${modeButton("pages", "pages")} ${modeButton("text", "paste text")}
          </div>
        </div>
        <form onSubmit=${submit}>
          <div>
            <label>date</label
            ><input
              name="date"
              type="date"
              value=${date}
              onInput=${(e) => setDate(e.currentTarget.value)}
            />
          </div>
          <div>
            <label>minutes</label
            ><input
              name="minutes"
              type="number"
              min="1"
              placeholder="estimated"
              value=${minutes}
              onInput=${(e) => setMinutes(e.currentTarget.value)}
            />
          </div>
          <div>
            <label>title</label
            ><input
              name="work"
              type="text"
              placeholder="本日は、お日柄もよく"
            />
          </div>
          <div>
            <label>source</label>
            <select
              name="source"
              value=${source}
              onChange=${(e) => setSource(e.currentTarget.value)}
            >
              <option value="book">book</option>
              <option value="article">article</option>
              <option value="manga">manga</option>
              <option value="other">other</option>
            </select>
          </div>
          ${
            mode === "pages"
              ? html`
                  <div>
                    <label>pages</label
                    ><input name="pages" type="number" min="0" step="0.5" />
                  </div>
                  <div>
                    <label>chars (overrides pages)</label
                    ><input name="chars" type="number" min="0" />
                  </div>
                `
              : html`
                  <div class="log-wide">
                    <label>url</label
                    ><input
                      name="url"
                      type="url"
                      placeholder="https://www3.nhk.or.jp/news/…"
                    />
                  </div>
                  <div class="log-wide">
                    <label>text read *</label>
                    <textarea
                      name="content"
                      rows="8"
                      required
                      placeholder="paste the article here"
                      value=${content}
                      onInput=${(e) => setContent(e.currentTarget.value)}
                    ></textarea>
                    <p class="log-count">${preview ?? "—"}</p>
                  </div>
                `
          }
          <div class="actions">
            <button type="submit" disabled=${busy}>
              ${busy ? "logging…" : "log"}
            </button>
            ${
              msg &&
              html`<span class="form-msg ${msg.ok ? "ok" : "error"}"
                >${msg.text}</span
              >`
            }
          </div>
        </form>
      </details>
    </div>
  `;
}
