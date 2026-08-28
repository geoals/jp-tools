// Editing a work's metadata, and switching which one is being read.
//
// `setCurrentWork` is the one write the reading pipeline depends on: the title
// it stores is stamped onto every line the logger captures next.
//
// **Adding a work is a title search, not a form.** It used to be: type the
// title, open vndb.org in another tab, find the entry, copy its id back, then
// open jiten.moe and copy a character count. Two of those three the app can do
// itself — `GET /api/works/search` asks VNDB by name and picks up the id and the
// cover — and the third is optional, so it is asked for on the progress bar that
// wants it rather than in the way of getting started.

import { html } from "htm/preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api.js";

const WORK_STATUSES = ["reading", "planned", "finished", "dropped"];

/** Set settings.current_work. Shared by the card's picker and the Library rows. */

export async function setCurrentWork(title) {
  await api("/api/settings", {
    method: "PUT",
    body: { current_work: title },
  });
}

/** Pick a new work by name.
 *
 *  The title that lands in the ledger is VNDB's Japanese one, which is also what
 *  the script and the window title say — so the work a line is stamped with is
 *  the work a reader would call it.
 *
 *  Typing a title nobody has heard of still works: **Use what I typed** creates
 *  the work with no VNDB entry behind it, which is the case for a doujin release
 *  or anything not a VN at all.
 */
export function WorkSearchForm({ onSaved, onCancel }) {
  const [q, setQ] = useState("");
  const [results, setResults] = useState(null);
  const [searching, setSearching] = useState(false);
  const [msg, setMsg] = useState(null);
  // The query the results belong to, so a slow answer for an older query cannot
  // land on a newer one.
  const latest = useRef("");

  useEffect(() => {
    const query = q.trim();
    latest.current = query;
    if (query.length < 2) {
      setResults(null);
      return;
    }
    // Debounced: VNDB is somebody else's server and this fires per keystroke.
    const t = setTimeout(async () => {
      setSearching(true);
      try {
        const res = await api(`/api/works/search?q=${encodeURIComponent(query)}`);
        if (latest.current === query) setResults(res.results ?? []);
      } catch (e) {
        if (latest.current === query) {
          setResults([]);
          setMsg({ ok: false, text: e.message });
        }
      } finally {
        if (latest.current === query) setSearching(false);
      }
    }, 350);
    return () => clearTimeout(t);
  }, [q]);

  async function create(title, vndbId) {
    setMsg(null);
    try {
      await api("/api/works", {
        method: "POST",
        body: { title, vndb_id: vndbId || undefined, status: "reading" },
      });
      // Reading it is why you added it. Anything else is a click in the library.
      await setCurrentWork(title);
      onSaved();
      onCancel?.();
    } catch (e) {
      setMsg({ ok: false, text: e.message });
    }
  }

  return html`
    <div class="work-search">
      <label for="work-search-input">What are you reading?</label>
      <input
        id="work-search-input"
        type="text"
        autofocus
        spellcheck="false"
        placeholder="type the title — Japanese or romanized"
        value=${q}
        onInput=${(e) => setQ(e.currentTarget.value)}
      />

      ${searching && html`<p class="settings-hint">searching vndb…</p>`}

      ${results?.length > 0 &&
      html`<ul class="work-search-results">
        ${results.map(
          (r) => html`
            <li key=${r.id}>
              <button type="button" onClick=${() => create(r.title, r.id)}>
                ${r.cover && html`<img src=${r.cover} alt="" loading="lazy" />`}
                <span class="work-search-titles">
                  <span class="work-search-title">${r.title}</span>
                  ${r.alt_title &&
                  html`<span class="work-search-alt">${r.alt_title}</span>`}
                  ${r.hours !== null &&
                  html`<span class="work-search-alt">
                    ${`about ${Math.round(r.hours)} hours`}
                  </span>`}
                </span>
              </button>
            </li>
          `,
        )}
      </ul>`}

      ${results?.length === 0 &&
      !searching &&
      html`<p class="settings-hint">Nothing on vndb by that name.</p>`}

      <div class="actions">
        ${q.trim() &&
        html`<button type="button" onClick=${() => create(q.trim(), null)}>
          Use “${q.trim()}”
        </button>`}
        ${onCancel &&
        html`<button type="button" class="ghost" onClick=${onCancel}>
          cancel
        </button>`}
        ${msg && html`<span class="form-msg error">${msg.text}</span>`}
      </div>
      <p class="settings-hint">
        Picking one fills in the cover for you. How long it is has to be pasted
        from jiten.moe — vndb has no character count — and Kotodex asks for that
        on the progress bar, once there is progress to show.
      </p>
    </div>
  `;
}

/** Metadata editor for one work, used to edit an existing one.
 *
 *  Editing PUTs by id so the title is never part of the update — retitling via
 *  POST would upsert a second row rather than rename, since the title is the
 *  join key lines are stamped with. Every field is prefilled from current
 *  metadata; status especially, because it is always sent and a blank select
 *  would silently reset a finished work to reading. */

export function WorkMetaForm({ work, onSaved, onCancel }) {
  const [msg, setMsg] = useState(null);
  const [busy, setBusy] = useState(false);
  const id = work?.meta?.id ?? null;

  async function save(e) {
    e.preventDefault();
    const f = e.currentTarget;
    const body = {
      vndb_id: f.vndb.value.trim() || undefined,
      total_chars: f.total.value ? Number(f.total.value) : undefined,
      status: f.status.value,
    };
    setBusy(true);
    setMsg(null);
    try {
      if (id !== null) {
        await api(`/api/works/${id}`, { method: "PUT", body });
      } else {
        await api("/api/works", {
          method: "POST",
          body: { ...body, title: f.title.value.trim() },
        });
      }
      setMsg({ ok: true, text: "saved ✓" });
      onSaved();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  return html`
    <form class="log work-meta-form" onSubmit=${save}>
      ${
        id === null &&
        html`<div>
          <label>title *</label
          ><input
            name="title"
            type="text"
            required
            placeholder="アイヨクノエウスティア"
          />
        </div>`
      }
      <div>
        <label>total characters</label
        ><input
          name="total"
          type="number"
          min="0"
          value=${work?.meta?.total_chars ?? ""}
          placeholder="from jiten.moe"
        />
      </div>
      <div>
        <label>cover art</label
        ><input name="vndb" type="text" placeholder="vndb link or id" />
      </div>
      <div>
        <label>status</label>
        <select name="status">
          ${WORK_STATUSES.map(
            (s) =>
              html`<option
                value=${s}
                selected=${(work?.meta?.status ?? "reading") === s}
              >
                ${s}
              </option>`,
          )}
        </select>
      </div>
      <div class="actions">
        <button type="submit" disabled=${busy}>${busy ? "…" : "save"}</button>
        ${
          onCancel &&
          html`<button type="button" class="ghost" onClick=${onCancel}>
            cancel
          </button>`
        }
        ${
          msg &&
          html`<span class="form-msg ${msg.ok ? "ok" : "error"}"
            >${msg.text}</span
          >`
        }
      </div>
    </form>
  `;
}
