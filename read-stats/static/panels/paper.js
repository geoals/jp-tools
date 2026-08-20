// Reading a book on paper, tracked against the epub of the same book.
//
// Two pieces, one flow. **PaperLog** is the sitting: type the last thing you
// read, look at what came back, save it. **AddPaperBook** is the setup: upload
// the epub, then say where the story starts and what pages the paper copy runs
// between. Both live on the library side of the app — a paper book is a work
// like any other, so its page is where the log sits and the shelf is where a
// book is added.
//
// **Preview and log are two calls and the offset travels between them.** The
// anchor search can land in the wrong place, and what gets saved has to be the
// span that was actually looked at, not a second search that might resolve
// differently.
//
// The same confirmed offset also feeds *skip*, which moves the position
// without writing a session. That is how a book already part-read when its
// epub is added gets caught up: those pages were read before there was
// anything to record them with, and logging them would credit a day that never
// happened.

import { html } from "htm/preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api.js";

function todayLocal() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const fmt = (n) => Math.round(n).toLocaleString("en");

/** The anchor log for one book: type where you stopped, confirm the find. */

export function PaperLog({ book, onLogged }) {
  const [anchor, setAnchor] = useState("");
  const [minutes, setMinutes] = useState("");
  const [date, setDate] = useState(todayLocal());
  const [preview, setPreview] = useState(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);

  // A different book is a different search: nothing from the last one applies.
  useEffect(() => {
    setAnchor("");
    setPreview(null);
    setMsg(null);
  }, [book.work]);

  async function search(from) {
    setBusy(true);
    setMsg(null);
    try {
      const r = await api("/api/books/preview", {
        method: "POST",
        body: {
          work: book.work,
          anchor,
          from,
          minutes: minutes ? Number(minutes) : undefined,
        },
      });
      setPreview(r);
    } catch (err) {
      setPreview(null);
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  async function skip() {
    setBusy(true);
    setMsg(null);
    try {
      await api("/api/books/skip", {
        method: "POST",
        body: { work: book.work, end: preview.found.end },
      });
      setMsg({ ok: true, text: "moved without logging it ✓" });
      setPreview(null);
      setAnchor("");
      onLogged();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    setBusy(true);
    setMsg(null);
    try {
      const r = await api("/api/books/log", {
        method: "POST",
        body: {
          work: book.work,
          end: preview.found.end,
          minutes: minutes ? Number(minutes) : undefined,
          date: date || undefined,
        },
      });
      const logged = `logged ${r.session.chars.toLocaleString("en")} chars ✓`;
      setMsg({ ok: true, text: logged });
      setPreview(null);
      setAnchor("");
      setMinutes("");
      onLogged();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  const w = preview?.words;
  const chars = preview ? `${preview.chars.toLocaleString("en")} chars` : "";
  const pages = preview?.pages ? `≈ ${preview.pages.toFixed(1)} pages` : null;
  const speed = preview?.speed ? `${fmt(preview.speed)} chars/h` : null;
  const wordLine = w
    ? `${fmt(w.new)} new · ${fmt(w.unknown)} unknown · ${fmt(w.seen)} unjudged · ${fmt(w.known)} known`
    : null;

  // Nothing logged yet, so this may well be a book already partway read.
  const untouched = book.position === book.body_start;

  return html`
    <div class="book-log">
      ${
        untouched &&
        html`<p class="book-catchup">
          Nothing logged for this book yet. If you are already partway through
          it, find the line you are on now and use
          <b>already read — don't log</b>, so the pages you read before this
          existed are not counted as today's.
        </p>`
      }
      <label class="book-anchor-label" for="book-anchor">
        Last thing you read — about ten characters, copied off the page
      </label>
      <textarea
        id="book-anchor"
        class="book-anchor"
        rows="2"
        placeholder="……そんなことを考えていた。"
        value=${anchor}
        onInput=${(e) => {
          setAnchor(e.currentTarget.value);
          setPreview(null);
        }}
      ></textarea>
      <div class="book-fields">
        <div>
          <label>minutes (optional)</label>
          <input
            type="number"
            min="1"
            placeholder="untimed"
            value=${minutes}
            onInput=${(e) => setMinutes(e.currentTarget.value)}
          />
        </div>
        <div>
          <label>date</label>
          <input
            type="date"
            value=${date}
            onInput=${(e) => setDate(e.currentTarget.value)}
          />
        </div>
        <div class="book-actions">
          <button
            type="button"
            disabled=${busy || anchor.trim().length < 2}
            onClick=${() => search(undefined)}
          >
            ${busy ? "searching…" : "find it"}
          </button>
        </div>
      </div>
      ${
        msg &&
        html`<p class=${`form-msg ${msg.ok ? "ok" : "error"}`}>${msg.text}</p>`
      }
      ${
        preview &&
        html`
          <div class="book-found">
            <p class="book-context">
              <span class="book-ctx-side">${preview.found.before}</span
              ><mark>${preview.found.matched}</mark
              ><span class="book-ctx-side">${preview.found.after}</span>
            </p>
            <p class="book-figures">
              ${[chars, pages, speed].filter(Boolean).join(" · ")}
            </p>
            ${wordLine && html`<p class="book-words">${wordLine}</p>`}
            ${
              preview.found.loose &&
              html`<p class="book-loose">
                Matched ignoring punctuation — check the highlighted text is
                where you stopped.
              </p>`
            }
            <div class="book-confirm">
              <button type="button" disabled=${busy} onClick=${save}>
                ${busy ? "saving…" : "log it"}
              </button>
              <button
                type="button"
                class="book-secondary"
                disabled=${busy}
                onClick=${() => search(preview.found.start + 1)}
              >
                not this one — keep looking
              </button>
              <button
                type="button"
                class="book-secondary"
                disabled=${busy}
                title="Move the bookmark here without recording a session"
                onClick=${skip}
              >
                already read — don't log
              </button>
            </div>
          </div>
        `
      }
    </div>
  `;
}

/** Adding a book: upload the epub, then say where the story starts.
 *
 *  With a `work` given, the epub attaches to that existing work (its page's
 *  "track on paper" path) and the title is not asked for; without one, the
 *  upload creates the work, title and all, which is the shelf's path. */

export function AddPaperBook({ work, onAdded, onDone }) {
  const [file, setFile] = useState(null);
  const [title, setTitle] = useState(work ?? "");
  const [added, setAdded] = useState(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);

  async function upload(e) {
    e.preventDefault();
    if (!file) return;
    setBusy(true);
    setMsg(null);
    try {
      const bytes = await file.arrayBuffer();
      const name = (work ?? title).trim();
      const r = await fetch(
        `/api/books/upload?title=${encodeURIComponent(name)}`,
        { method: "POST", body: bytes },
      );
      if (!r.ok) throw new Error(await r.text());
      setAdded(await r.json());
      onAdded();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  async function setup(e) {
    e.preventDefault();
    const f = e.currentTarget;
    setBusy(true);
    setMsg(null);
    try {
      const r = await api("/api/books/setup", {
        method: "POST",
        body: {
          work: added.book.work,
          anchor: f.anchor.value,
          first_page: f.first.value ? Number(f.first.value) : undefined,
          last_page: f.last.value ? Number(f.last.value) : undefined,
        },
      });
      const cpp = r.chars_per_page
        ? `, ${fmt(r.chars_per_page)} chars per page`
        : "";
      setMsg({
        ok: true,
        text: `ready — ${r.body_chars.toLocaleString("en")} characters of body text${cpp}`,
      });
      setAdded(null);
      setFile(null);
      setTitle("");
      onAdded?.();
      onDone?.();
    } catch (err) {
      setMsg({ ok: false, text: err.message });
    } finally {
      setBusy(false);
    }
  }

  return html`
    <details class="log" open=${added !== null}>
      <summary>Add a book</summary>
        ${
          added === null
            ? html`
                <form onSubmit=${upload}>
                  <div class="log-wide">
                    <label>epub *</label>
                    <input
                      type="file"
                      accept=".epub"
                      required
                      onChange=${(e) => {
                        const f = e.currentTarget.files?.[0] ?? null;
                        setFile(f);
                        if (f && !title)
                          setTitle(f.name.replace(/\.epub$/i, ""));
                      }}
                    />
                  </div>
                  ${
                    !work &&
                    html`<div class="log-wide">
                      <label
                        >title * — exactly how it should be named
                        everywhere</label
                      >
                      <input
                        type="text"
                        required
                        value=${title}
                        onInput=${(e) => setTitle(e.currentTarget.value)}
                      />
                    </div>`
                  }
                  <div class="actions">
                    <button type="submit" disabled=${busy || !file}>
                      ${busy ? "reading…" : "upload"}
                    </button>
                  </div>
                </form>
              `
            : html`
                <p class="book-setup-hint">
                  The epub opens like this. Copy the first line of the story
                  into the box below — everything before it is front matter and
                  is never counted.
                </p>
                <pre class="book-head">${added.head}</pre>
                <form onSubmit=${setup}>
                  <div class="log-wide">
                    <label>first line of the story *</label>
                    <input name="anchor" type="text" required />
                  </div>
                  <div>
                    <label>first page of the body</label>
                    <input name="first" type="number" min="1" />
                  </div>
                  <div>
                    <label>last page of the body</label>
                    <input name="last" type="number" min="1" />
                  </div>
                  <div class="actions">
                    <button type="submit" disabled=${busy}>
                      ${busy ? "saving…" : "set up"}
                    </button>
                  </div>
                </form>
              `
        }
        ${
          msg &&
          html`<p class=${`form-msg ${msg.ok ? "ok" : "error"}`}>
            ${msg.text}
          </p>`
        }
    </details>
  `;
}
