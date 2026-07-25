// Logging reading a texthooker cannot see: physical books, articles, ebooks.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";

export function LogForm({ onLogged }) {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);

  async function submit(e) {
    e.preventDefault();
    const f = e.currentTarget;
    const body = {
      date: f.date.value || undefined,
      minutes: Number(f.minutes.value),
      pages: f.pages.value ? Number(f.pages.value) : undefined,
      chars: f.chars.value ? Number(f.chars.value) : undefined,
      work: f.work.value || undefined,
      source: f.source.value,
    };
    setBusy(true);
    setMsg(null);
    try {
      await api("/api/sessions", { method: "POST", body });
      setMsg({ ok: true, text: "logged ✓" });
      f.minutes.value = "";
      f.pages.value = "";
      f.chars.value = "";
      onLogged();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  return html`
    <div class="card">
      <details class="log">
        <summary>Log reading (physical book, manga, anything unhooked)</summary>
        <form onSubmit=${submit}>
          <div><label>date</label><input name="date" type="date" /></div>
          <div>
            <label>minutes *</label
            ><input name="minutes" type="number" min="1" required />
          </div>
          <div>
            <label>pages</label
            ><input name="pages" type="number" min="0" step="0.5" />
          </div>
          <div>
            <label>chars (overrides pages)</label
            ><input name="chars" type="number" min="0" />
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
            <select name="source">
              <option value="book">book</option>
              <option value="manga">manga</option>
              <option value="other">other</option>
            </select>
          </div>
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
