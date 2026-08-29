// Editing a work's metadata, and switching which one is being read.
//
// `setCurrentWork` is the one write the reading pipeline depends on: the title
// it stores is stamped onto every line the logger captures next. The empty
// string is a value it takes: nothing is being read, and lines captured until
// something is get stamped with no title at all.
//
// Everything a work *is* is edited in one form — length, cover, status, and the
// window the game draws in. The window used to be a second form glued under the
// dialog with a save button of its own, which is how a reader could set the
// three fields, close, and find the capture still pointing at the last VN.
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

/** The VNDB title search: a box, a debounced query, and the results as a list.
 *
 *  Controlled, so the caller owns the text. Adding a work needs it for **Use
 *  what I typed**, and the cover picker seeds it with the title the work
 *  already has.
 */
function VndbSearch({ q, onQ, onPick, label, focus }) {
  const [results, setResults] = useState(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState(null);
  const box = useRef(null);
  // The query the results belong to, so a slow answer for an older query cannot
  // land on a newer one.
  const latest = useRef("");

  // The attribute alone does not fire for a node inserted into a live tree,
  // which is every way this form is ever opened.
  useEffect(() => {
    if (focus) box.current?.focus();
  }, [focus]);

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
          setError(e.message);
        }
      } finally {
        if (latest.current === query) setSearching(false);
      }
    }, 350);
    return () => clearTimeout(t);
  }, [q]);

  return html`
    <label for="work-search-input">${label}</label>
    <input
      id="work-search-input"
      ref=${box}
      type="text"
      spellcheck="false"
      placeholder="search for the title — Japanese or romanized"
      value=${q}
      onInput=${(e) => onQ(e.currentTarget.value)}
    />

    ${searching && html`<p class="settings-hint">searching vndb…</p>`}
    ${error && html`<p class="form-msg error">${error}</p>`}

    ${results?.length > 0 &&
    html`<ul class="work-search-results">
      ${results.map(
        (r) => html`
          <li key=${r.id}>
            <button type="button" onClick=${() => onPick(r.title, r.id)}>
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
  `;
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
export function WorkSearchForm({ settings, onSaved, onCancel }) {
  const [q, setQ] = useState("");
  // Adding the *first* work is starting to read it — there is nothing else it
  // could mean, and asking would be a question with one answer. Adding while
  // something is already open is shelving: the library is as much a list of
  // what comes next as of what is being read now, and switching the capture
  // target out from under a session is not what "+" was pressed for.
  const [start, setStart] = useState(!settings?.current_work);
  const [msg, setMsg] = useState(null);

  async function create(title, vndbId) {
    setMsg(null);
    try {
      await api("/api/works", {
        method: "POST",
        body: {
          title,
          vndb_id: vndbId || undefined,
          status: start ? "reading" : "planned",
        },
      });
      if (start) await setCurrentWork(title);
      onSaved();
      onCancel?.();
    } catch (e) {
      setMsg({ ok: false, text: e.message });
    }
  }

  return html`
    <div class="work-search">
      <${VndbSearch}
        q=${q}
        onQ=${setQ}
        onPick=${create}
        label="What are you reading?"
        focus=${true}
      />

      <label class="work-search-start">
        <input
          type="checkbox"
          checked=${start}
          onChange=${(e) => setStart(e.currentTarget.checked)}
        />
        Start reading it now
      </label>

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
    </div>
  `;
}

/** The open windows and the one in front, for pointing capture at the game.
 *
 *  Fetched when the form mounts rather than held on the dashboard's poll: the
 *  reader's next move after seeing "not set" is to click the game and come
 *  back, and a list taken a minute ago will not have it in.
 */
function useOpenWindows() {
  const [windows, setWindows] = useState([]);
  const [focused, setFocused] = useState(null);
  useEffect(() => {
    api("/api/vn/windows")
      .then((r) => {
        setWindows(r.windows || []);
        setFocused(r.focused || null);
      })
      .catch(() => {});
  }, []);
  return { windows, focused };
}

/** Whether what is in the box matches a window that is actually open. A stale
 *  title still mines — it screenshots the wrong thing, silently, which is the
 *  one fault here nothing else reports. */
function windowHint(name, windows) {
  if (!name) {
    return "Pick the game's window so the overlay can follow it.";
  }
  if (!windows.length) return "";
  return windows.includes(name)
    ? "Attached."
    : "Not open right now. Pick it again once the game is running.";
}

/** Metadata editor for one work: everything the work *is*, saved together.
 *
 *  Editing PUTs by id so the title is never part of the update — retitling via
 *  POST would upsert a second row rather than rename, since the title is the
 *  join key lines are stamped with. Every field is prefilled from current
 *  metadata; status especially, because it is always sent and a blank select
 *  would silently reset a finished work to reading.
 *
 *  The window is here rather than beside the capture controls because it is a
 *  property of the work — each game draws in its own window, and switching VNs
 *  switches the target with it. */

export function WorkMetaForm({ work, isCurrent, onSaved, onCancel, onDeleted }) {
  const [msg, setMsg] = useState(null);
  const [busy, setBusy] = useState(false);
  const { windows, focused } = useOpenWindows();
  const [vnWindow, setVnWindow] = useState(work?.meta?.vn_window ?? "");
  // null is "leave the cover alone", "" is "remove it", an id is "fetch that
  // one". The field has three states because the server's does.
  const [cover, setCover] = useState(null);
  const [picking, setPicking] = useState(false);
  const [coverQ, setCoverQ] = useState(work?.work ?? "");
  const id = work?.meta?.id ?? null;

  async function save(e) {
    e.preventDefault();
    const f = e.currentTarget;
    const body = {
      vndb_id: cover ?? undefined,
      total_chars: f.total.value ? Number(f.total.value) : undefined,
      status: f.status.value,
      // Always sent, empty included: emptying the box is how the window is
      // cleared, and an undefined would make that the one edit the form
      // cannot make.
      vn_window: vnWindow.trim(),
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

  const hint = windowHint(vnWindow, windows);
  // A book has no window to attach to and no VNDB entry to take a cover from.
  // `kind` is what `/api/works` derives from where a title's text came from, so
  // a work added through the VN search answers `vn` before it has been read.
  const isBook = work?.kind === "book";

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
      ${!isBook &&
      html`<div class="work-cover-field">
        <label>cover art</label>
        <div class="work-cover-row">
          ${
            work?.meta?.cover && cover !== ""
              ? html`<img class="work-cover-thumb" src=${work.meta.cover} alt="" />`
              : html`<span class="meta-hint">none</span>`
          }
          <button
            type="button"
            class="ghost"
            onClick=${() => setPicking(!picking)}
          >
            ${picking ? "cancel" : "find on vndb"}
          </button>
          ${
            work?.meta?.cover &&
            cover !== "" &&
            html`<button type="button" class="ghost" onClick=${() => setCover("")}>
              remove
            </button>`
          }
        </div>
        ${
          // The same search adding a work uses, seeded with the title this one
          // already has. A box wanting "v3144" made the reader go and find it,
          // which is the errand the search was built to end.
          picking &&
          html`<div class="work-search">
            <${VndbSearch}
              q=${coverQ}
              onQ=${setCoverQ}
              onPick=${(_title, vndbId) => {
                setCover(vndbId);
                setPicking(false);
              }}
              label="Which one is it?"
              focus=${true}
            />
          </div>`
        }
        ${
          cover !== null &&
          html`<div class="meta-hint">
            ${cover ? "New cover on save." : "Cover removed on save."}
          </div>`
        }
      </div>`}
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
      ${!isBook &&
      html`<div class="work-window-field">
        <label for="vn-window-input">game window</label>
        <select
          id="vn-window-input"
          value=${vnWindow}
          onChange=${(e) => setVnWindow(e.currentTarget.value)}
        >
          <option value="">— not set —</option>
          ${
            // Kept in the list even when the game is not running: it is the
            // setting's value, and dropping it would make closing the game look
            // like losing it.
            vnWindow &&
            !windows.includes(vnWindow) &&
            html`<option value=${vnWindow}>${`${vnWindow} (not open)`}</option>`
          }
          ${windows.map(
            // The window in front is the answer on a good day: the reader was
            // looking at the game a moment ago. Said in the list rather than as
            // a button beside it, which was a second control for one choice.
            (w) => html`<option value=${w}>
              ${w === focused ? `${w} — in front` : w}
            </option>`,
          )}
        </select>
        ${hint && html`<div class="meta-hint">${hint}</div>`}
      </div>`}
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
      ${
        onDeleted &&
        html`<${RemoveWork}
          work=${work}
          isCurrent=${isCurrent}
          onDeleted=${onDeleted}
        />`
      }
    </form>
  `;
}

/** Taking a work off the shelf.
 *
 *  What it removes is the *metadata* — the cover, the length, the status, the
 *  window — because that is all a `works` row is. The reading is stamped with
 *  the title, not with the row, so a work that has been read keeps its lines,
 *  its sittings and its place in every figure, and comes back on the shelf as a
 *  title with no cover. A work with nothing read under it disappears outright,
 *  since the row was the only thing that said it existed.
 *
 *  Saying which of those two it will be is the whole point of the confirm step:
 *  "remove" meaning two different things depending on whether you have read any
 *  of it is exactly the surprise worth spending a click on. */
function RemoveWork({ work, isCurrent, onDeleted }) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);
  const id = work?.meta?.id ?? null;
  if (id === null) return null;
  const read = (work?.chars ?? 0) > 0;

  async function remove() {
    setBusy(true);
    setMsg(null);
    try {
      await api(`/api/works/${id}`, { method: "DELETE" });
      onDeleted();
    } catch (e) {
      setMsg(e.message);
      setBusy(false);
    }
  }

  if (!confirming) {
    return html`<div class="work-danger">
      <div class="work-danger-row">
        <button type="button" class="ghost" onClick=${() => setConfirming(true)}>
          remove from library
        </button>
      </div>
    </div>`;
  }

  return html`
    <div class="work-danger">
      <div class="meta-hint">
        ${
          read
            ? "Removes the cover, length, status and window. The reading stays: this title keeps its lines and its sittings, and stays on the shelf with nothing filled in."
            : "Nothing has been read under this title, so it goes entirely."
        }
      </div>
      ${
        isCurrent &&
        html`<div class="meta-hint">
          This is what you are reading — removing it stops reading it.
        </div>`
      }
      <div class="work-danger-row">
        <button type="button" class="danger" disabled=${busy} onClick=${remove}>
          ${busy ? "…" : "remove"}
        </button>
        ${
          read &&
          html`<button
            type="button"
            class="ghost"
            onClick=${() =>
              setMsg("Not implemented — the lines stay in the ledger.")}
          >
            delete the reading too
          </button>`
        }
        <button
          type="button"
          class="ghost"
          onClick=${() => setConfirming(false)}
        >
          cancel
        </button>
        ${msg && html`<span class="form-msg error">${msg}</span>`}
      </div>
    </div>
  `;
}
