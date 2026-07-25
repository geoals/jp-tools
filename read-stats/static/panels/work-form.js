// Editing a work's metadata, and switching which one is being read.
//
// `setCurrentWork` is the one write the reading pipeline depends on: the title
// it stores is stamped onto every line the logger captures next.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";

const WORK_STATUSES = ["reading", "queued", "finished", "dropped"];

/** Set settings.current_work. Shared by the card's picker and the Library rows. */

export async function setCurrentWork(title) {
  await api("/api/settings", {
    method: "PUT",
    body: { current_work: title },
  });
}

/** Metadata editor for one work, used both to add a work and to edit one.
 *
 *  Editing an existing work PUTs by id so the title is never part of the
 *  update — retitling via POST would upsert a second row rather than rename,
 *  since the title is the join key lines are stamped with. Every field is
 *  prefilled from current metadata; status especially, because it is always
 *  sent and a blank select would silently reset a finished work to reading. */

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
          placeholder="from jpdb"
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
