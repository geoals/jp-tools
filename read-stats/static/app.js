import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { html } from "htm/preact";
import { api } from "./api.js";
import { Reader } from "./reader.js";
import {
  GoalMeter,
  DayTimelineChart,
  MinutesBarChart,
  ProgressBar,
  RateTrendChart,
  SpeedTrendChart,
} from "./charts.js";

const REFRESH_MS = 60_000;

function fmtMins(secs) {
  const mins = Math.round(secs / 60);
  return mins < 100
    ? `${mins} min`
    : `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, "0")}m`;
}

function fmtChars(n) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  return n >= 10_000 ? `${(n / 1000).toFixed(1)}k` : n.toLocaleString("en");
}

/** Mean chars/day over the trailing 7 *complete* days (today excluded — it
 *  would drag the average down all morning), zero days included, clipped to
 *  the pace_start_date cutoff so a reading break doesn't pollute the window.
 *  Falls back to today's partial data when the window is otherwise empty.
 *
 *  Returns the window alongside the mean so the finish estimate can explain
 *  itself in a tooltip. */
function paceCharsPerDay(days, settings) {
  let win = days.slice(-8, -1);
  if (settings.pace_start_date) {
    win = win.filter((d) => d.date >= settings.pace_start_date);
  }
  let partial = false;
  if (!win.length) {
    win = days.slice(-1);
    partial = true;
  }
  if (!win.length) return { pace: 0, days: 0, partial: false };
  return {
    pace: win.reduce((a, d) => a + d.chars, 0) / win.length,
    days: win.length,
    partial,
  };
}

function fmtFinishDate(daysLeft) {
  const d = new Date(Date.now() + daysLeft * 86_400_000);
  const opts = { month: "short", day: "numeric" };
  if (d.getFullYear() !== new Date().getFullYear()) opts.year = "numeric";
  return `≈ ${d.toLocaleDateString("en", opts)}`;
}

function TodayCard({ summary }) {
  const { today, goal } = summary;
  const mins = today.active_secs / 60;
  const speed =
    today.active_secs >= 600 ? today.chars / (today.active_secs / 3600) : null;
  // Same 10-minute floor as speed: below it the per-hour denominator is noise.
  const lookupsPerHour =
    today.active_secs >= 600
      ? today.lookups / (today.active_secs / 3600)
      : null;
  // Sub-values are built as strings so prettier can't reflow the markup and
  // change the rendered spacing.
  const lookupRate =
    lookupsPerHour !== null ? `(${lookupsPerHour.toFixed(1)}/h)` : null;
  const bestStretch =
    today.focus.longest_stretch_secs > 0
      ? `(${fmtMins(today.focus.longest_stretch_secs)} best)`
      : null;
  return html`
    <div class="card">
      <h2>Today · ${today.date}</h2>
      <div class="hero-row">
        <span class="hero">${fmtMins(today.active_secs)}</span>
        <span class="hero-sub">
          ${mins >= goal.target_mins
            ? html`<span class="goal-met">target met</span>`
            : mins >= goal.floor_mins
              ? html`<span class="goal-met">floor met</span> ·
                  ${Math.ceil(goal.target_mins - mins)} min to target`
              : `${Math.ceil(goal.floor_mins - mins)} min to floor`}
        </span>
      </div>
      <${GoalMeter}
        mins=${mins}
        floorMins=${goal.floor_mins}
        targetMins=${goal.target_mins}
      />
      <div class="meter-caption">
        <span>0</span><span>floor ${goal.floor_mins}</span
        ><span>${goal.target_mins} min</span>
      </div>
      <div class="tile-row">
        <div class="tile">
          <div class="label">characters</div>
          <div class="value">${today.chars.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">speed</div>
          <div class="value">
            ${speed ? `${fmtChars(Math.round(speed))}/h` : "—"}
          </div>
        </div>
        <div class="tile">
          <div class="label">cards mined</div>
          <div class="value">${today.cards > 0 ? today.cards : "—"}</div>
        </div>
        <div class="tile">
          <div class="label">lookups</div>
          <div class="value">
            ${today.lookups > 0 ? today.lookups.toLocaleString("en") : "—"}
            ${lookupRate && html`<span class="value-sub">${lookupRate}</span>`}
          </div>
        </div>
        <div class="tile">
          <div class="label">focus</div>
          <div class="value">
            ${today.focus.ratio !== null
              ? `${Math.round(today.focus.ratio * 100)}%`
              : "—"}
            ${bestStretch &&
            html`<span class="value-sub">${bestStretch}</span>`}
          </div>
        </div>
      </div>
    </div>
  `;
}

function WeekTiles({ days }) {
  const week = days.slice(-7);
  const chars = week.reduce((a, d) => a + d.chars, 0);
  const secs = week.reduce((a, d) => a + d.active_secs, 0);
  const daysMet = week.filter((d) => d.active_secs > 0).length;
  return html`
    <div class="card">
      <h2>Last 7 days</h2>
      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">time</div>
          <div class="value">${fmtMins(secs)}</div>
        </div>
        <div class="tile">
          <div class="label">characters</div>
          <div class="value">${chars.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">days read</div>
          <div class="value">${daysMet}/7</div>
        </div>
        <div class="tile">
          <div class="label">avg speed</div>
          <div class="value">
            ${secs >= 600
              ? `${fmtChars(Math.round(chars / (secs / 3600)))}/h`
              : "—"}
          </div>
        </div>
      </div>
    </div>
  `;
}

/** Progress numbers for one work; needs a jpdb total_chars to say anything. */
function workProgress(w, days, settings) {
  const total = w?.meta?.total_chars;
  if (!total) return null;
  const workSpeed =
    w.active_secs >= 600 ? w.chars / (w.active_secs / 3600) : null;
  const remaining = Math.max(0, total - w.chars);
  const { pace, days: paceDays, partial } = paceCharsPerDay(days, settings);
  const window = partial
    ? "today so far (no complete day in the window yet)"
    : `the last ${paceDays} complete day${paceDays === 1 ? "" : "s"}, today excluded`;
  const cutoff = settings.pace_start_date
    ? ` Days before ${settings.pace_start_date} are excluded.`
    : "";
  return {
    pct: Math.min(100, (w.chars / total) * 100),
    caption: `${fmtChars(w.chars)} / ${fmtChars(total)} chars`,
    remaining,
    hoursLeft: workSpeed ? remaining / workSpeed : null,
    finish: pace > 0 && remaining > 0 ? fmtFinishDate(remaining / pace) : null,
    finishHint:
      pace > 0 && remaining > 0
        ? `${fmtChars(remaining)} chars left ÷ ${fmtChars(Math.round(pace))} chars/day, ` +
          `your average across all works over ${window} (zero days counted).${cutoff}`
        : null,
  };
}

const WORK_STATUSES = ["reading", "queued", "finished", "dropped"];

/** Set settings.current_work. Shared by the card's picker and the Library rows. */
async function setCurrentWork(title) {
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
function WorkMetaForm({ work, onSaved, onCancel }) {
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
      ${id === null &&
      html`<div>
        <label>title *</label
        ><input
          name="title"
          type="text"
          required
          placeholder="アイヨクノエウスティア"
        />
      </div>`}
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
              html`<option value=${s} selected=${(work?.meta?.status ?? "reading") === s}>
                ${s}
              </option>`,
          )}
        </select>
      </div>
      <div class="actions">
        <button type="submit" disabled=${busy}>${busy ? "…" : "save"}</button>
        ${onCancel &&
        html`<button type="button" class="ghost" onClick=${onCancel}>
          cancel
        </button>`}
        ${msg &&
        html`<span class="form-msg ${msg.ok ? "ok" : "error"}"
          >${msg.text}</span
        >`}
      </div>
    </form>
  `;
}

function CurrentReading({ works, settings, days, onSaved }) {
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [winBusy, setWinBusy] = useState(false);
  const [windows, setWindows] = useState([]);

  // Fetched once on mount rather than with the 60s refresh: it shells out to
  // xdotool, and the window list only changes when you launch a different VN.
  useEffect(() => {
    api("/api/vn/windows")
      .then((r) => setWindows(r.windows || []))
      .catch(() => setWindows([]));
  }, []);

  const title = settings.current_work;
  const current = title ? works.find((w) => w.work === title) : null;
  const meta = current?.meta;
  const prog = workProgress(current, days, settings);
  // A title that matches no work at all: usually a typo, or a work the
  // tracker hasn't stamped any lines with yet. Worth saying out loud —
  // silently rendering an empty card just looks broken.
  const unmatched = title && !current;

  async function pick(e) {
    e.preventDefault();
    setBusy(true);
    try {
      await setCurrentWork(e.currentTarget.work.value.trim());
      onSaved();
    } catch (err) {
      alert(err.message);
    } finally {
      setBusy(false);
    }
  }

  async function saveWindow(e) {
    e.preventDefault();
    setWinBusy(true);
    try {
      await api("/api/settings", {
        method: "PUT",
        body: { vn_window: e.currentTarget.vnwindow.value.trim() },
      });
      onSaved();
    } catch (err) {
      alert(err.message);
    } finally {
      setWinBusy(false);
    }
  }

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Currently reading</h2>
        ${current &&
        html`<button class="ghost" onClick=${() => setEditing((v) => !v)}>
          ${editing ? "close" : "edit"}
        </button>`}
      </div>
      ${current &&
      prog &&
      html`
        <div class="current-work">
          ${meta.cover &&
          html`<img class="cover" src=${meta.cover} alt="cover" />`}
          <div class="info">
            <div class="title">${current.work}</div>
            <${ProgressBar}
              pct=${prog.pct}
              label="Progress through ${current.work}"
            />
            <div class="progress-caption">
              <span>${prog.caption}</span>
              <span>${prog.pct.toFixed(1)}%</span>
            </div>
            <div class="tile-row">
              ${prog.remaining !== null &&
              html`
                <div class="tile">
                  <div class="label">remaining</div>
                  <div class="value">${fmtChars(prog.remaining)}</div>
                </div>
              `}
              <div class="tile">
                <div class="label">time left</div>
                <div class="value">
                  ${prog.hoursLeft !== null
                    ? `${prog.hoursLeft < 10 ? prog.hoursLeft.toFixed(1) : Math.round(prog.hoursLeft)} h`
                    : "—"}
                </div>
              </div>
              <div
                class=${prog.finishHint ? "tile has-hint" : "tile"}
                title=${prog.finishHint ??
                "No estimate: needs both a remaining count and a non-zero recent pace."}
              >
                <div class="label">finish</div>
                <div class="value">${prog.finish ?? "—"}</div>
              </div>
            </div>
          </div>
        </div>
      `}
      ${current &&
      !prog &&
      html`
        <div class="current-work">
          <div class="info">
            <div class="title">${current.work}</div>
            <div class="meta-hint">
              ${fmtChars(current.chars)} read so far. No total length set — add
              the jpdb character count with <strong>edit</strong> to get
              progress, hours left and a finish date.
            </div>
          </div>
        </div>
      `}
      ${unmatched &&
      html`
        <div class="meta-hint">
          Nothing tracked for <strong>${title}</strong> yet. If you have been
          reading it, the title here has to match the one your tracker stamps
          on lines exactly — pick from the list below instead of typing it.
        </div>
      `}
      ${!title &&
      html`<div class="meta-hint">
        No work selected. Pick one below, or set one from the Library.
      </div>`}
      ${editing &&
      html`<${WorkMetaForm}
        work=${current}
        onSaved=${onSaved}
        onCancel=${() => setEditing(false)}
      />`}
      <form class="now-reading" onSubmit=${pick}>
        <label for="now-reading-input">Switch to</label>
        <div class="now-reading-row">
          <input
            id="now-reading-input"
            name="work"
            type="text"
            list="known-works"
            value=${title}
            placeholder="pick a work, or type a new title"
          />
          <datalist id="known-works">
            ${works.map((w) => html`<option value=${w.work}></option>`)}
          </datalist>
          <button type="submit" disabled=${busy}>${busy ? "…" : "set"}</button>
        </div>
      </form>
      <form class="now-reading" onSubmit=${saveWindow}>
        <label for="vn-window-input">VN window</label>
        <div class="now-reading-row">
          <input
            id="vn-window-input"
            name="vnwindow"
            type="text"
            list="open-windows"
            value=${settings.vn_window}
            placeholder="pick the VN's window"
          />
          <datalist id="open-windows">
            ${windows.map((w) => html`<option value=${w}></option>`)}
          </datalist>
          <button type="submit" disabled=${winBusy}>
            ${winBusy ? "…" : "set"}
          </button>
        </div>
        <div class="meta-hint">${vnWindowHint(settings, windows)}</div>
      </form>
    </div>
  `;
}

/** Why this box exists, and whether what's in it currently matches a real
 *  window — a stale title still mines, it just screenshots the wrong thing. */
function vnWindowHint(settings, windows) {
  const set = settings.vn_window;
  if (!set) {
    return "Not set — the mine button screenshots whatever has focus, which is the browser when you mine from this machine. Pick the VN's window above.";
  }
  const matches = windows.some((w) => w.includes(set));
  return matches
    ? `Screenshots match "${set}". Update it when you switch VNs.`
    : `No open window matches "${set}" — captures will fall back to the focused window. Re-pick it if you have switched VNs.`;
}

/** The library is where works are managed: switch the current one, edit
 *  metadata, add one you haven't started. The Currently-reading card stays
 *  read-only status so the two don't compete. */
function WorksTable({ works, settings, onSaved }) {
  // Keyed by title rather than index so the open editor follows its row when
  // the list re-sorts on refresh (it sorts by last_read).
  const [editing, setEditing] = useState(null);
  const [adding, setAdding] = useState(false);

  async function makeCurrent(title) {
    try {
      await setCurrentWork(title);
      onSaved();
    } catch (err) {
      alert(err.message);
    }
  }

  return html`
    <div class="card">
      <div class="card-head">
        <h2>Library</h2>
        <button class="ghost" onClick=${() => setAdding((v) => !v)}>
          ${adding ? "close" : "add work"}
        </button>
      </div>
      ${adding &&
      html`<${WorkMetaForm}
        work=${null}
        onSaved=${() => {
          setAdding(false);
          onSaved();
        }}
        onCancel=${() => setAdding(false)}
      />`}
      ${works.length === 0
        ? html`<div class="meta-hint">
            No works yet — add one above, or just start reading and the tracker
            will stamp lines with a title.
          </div>`
        : html`<table class="days">
            <thead>
              <tr>
                <th>title</th>
                <th>time</th>
                <th>chars</th>
                <th>progress</th>
                <th>last read</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              ${works.slice(0, 10).map((w) => {
                const isCurrent = w.work === settings.current_work;
                return html`
                  <tr class=${isCurrent ? "row-current" : ""}>
                    <td class="work-name">
                      ${w.work ?? "(unlabeled)"}
                      ${isCurrent && html`<span class="status-tag">current</span>`}
                      ${w.meta &&
                      w.meta.status !== "reading" &&
                      html`<span class="status-tag">${w.meta.status}</span>`}
                    </td>
                    <td>${w.active_secs > 0 ? fmtMins(w.active_secs) : "—"}</td>
                    <td>${w.chars > 0 ? w.chars.toLocaleString("en") : "—"}</td>
                    <td>
                      ${w.meta?.total_chars
                        ? `${Math.min(100, (w.chars / w.meta.total_chars) * 100).toFixed(0)}%`
                        : "—"}
                    </td>
                    <td>${w.last_read ?? "—"}</td>
                    <td class="row-actions">
                      ${!isCurrent &&
                      w.work &&
                      html`<button
                        class="ghost"
                        onClick=${() => makeCurrent(w.work)}
                      >
                        read
                      </button>`}
                      ${w.meta &&
                      html`<button
                        class="ghost"
                        onClick=${() =>
                          setEditing(editing === w.work ? null : w.work)}
                      >
                        ${editing === w.work ? "close" : "edit"}
                      </button>`}
                    </td>
                  </tr>
                  ${editing === w.work &&
                  html`<tr>
                    <td colspan="6" class="work-editor-cell">
                      <${WorkMetaForm}
                        work=${w}
                        onSaved=${onSaved}
                        onCancel=${() => setEditing(null)}
                      />
                    </td>
                  </tr>`}
                `;
              })}
            </tbody>
          </table>`}
    </div>
  `;
}

function LogForm({ onLogged }) {
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
            ${msg &&
            html`<span class="form-msg ${msg.ok ? "ok" : "error"}"
              >${msg.text}</span
            >`}
          </div>
        </form>
      </details>
    </div>
  `;
}

function SessionsToday({ sessions }) {
  if (!sessions) return null;
  const rows = [
    ...sessions.derived.map((s) => ({ ...s, kind: "vn" })),
    ...sessions.manual.map((s) => ({
      ...s,
      kind: s.source,
      active_secs: s.end_ts - s.start_ts,
    })),
  ].sort((a, b) => a.start_ts - b.start_ts);
  if (!rows.length) return null;
  const hhmm = (ts) => new Date(ts * 1000).toTimeString().slice(0, 5);
  return html`
    <div class="card">
      <h2>Today's sessions</h2>
      <table class="days">
        <thead>
          <tr>
            <th>time</th>
            <th>mins</th>
            <th>chars</th>
            <th>chars/h</th>
            <th>cards</th>
            <th>cards/h</th>
          </tr>
        </thead>
        <tbody>
          ${rows.map((s) => {
            const hours = s.active_secs / 3600;
            return html`
              <tr>
                <td>
                  ${hhmm(s.start_ts)}–${hhmm(s.end_ts)}
                  <span class="status-tag">${s.kind}</span>
                </td>
                <td>${Math.round(s.active_secs / 60)}</td>
                <td>${s.chars.toLocaleString("en")}</td>
                <td>
                  ${s.active_secs >= 600
                    ? fmtChars(Math.round(s.chars / hours))
                    : "—"}
                </td>
                <td>${s.cards > 0 ? s.cards : "—"}</td>
                <td>
                  ${s.cards > 0 && s.active_secs >= 600
                    ? (s.cards / hours).toFixed(1)
                    : "—"}
                </td>
              </tr>
            `;
          })}
        </tbody>
      </table>
    </div>
  `;
}

/** Smoothing windows offered by the granularity slider, in minutes. */
const SMOOTH_STEPS = [1, 2, 3, 5, 8, 12, 20, 30, 45];

/**
 * One day, minute by minute. Owns the date and the smoothing window.
 *
 * The buckets arrive raw (one minute) and are smoothed in the browser, so
 * dragging the granularity slider is instant and issues no request — which
 * also keeps it off the server while a reading session is live.
 */
function DayDetailCard({ todayDate }) {
  const [date, setDate] = useState(todayDate);
  const [smoothIdx, setSmoothIdx] = useState(3); // 5 min
  const [data, setData] = useState(null);
  const [err, setErr] = useState(null);

  useEffect(() => {
    let live = true;
    setData(null);
    setErr(null);
    api(`/api/day/timeline?date=${date}`)
      .then((d) => live && setData(d))
      .catch((e) => live && setErr(e.message));
    return () => {
      live = false;
    };
  }, [date]);

  const shift = (days) => {
    const d = new Date(`${date}T12:00:00`);
    d.setDate(d.getDate() + days);
    setDate(d.toISOString().slice(0, 10));
  };

  const windowMins = SMOOTH_STEPS[smoothIdx];
  const sessions = data?.sessions ?? [];
  const totalMins = sessions.reduce((a, s) => a + s.active_secs, 0) / 60;

  return html`
    <div class="card">
      <h2>Day detail · reading speed vs. lookups</h2>
      <div class="day-controls">
        <button class="day-nav" onClick=${() => shift(-1)} title="Previous day">◀</button>
        <input
          type="date"
          value=${date}
          max=${todayDate}
          onInput=${(e) => e.target.value && setDate(e.target.value)}
        />
        <button
          class="day-nav"
          onClick=${() => shift(1)}
          disabled=${date >= todayDate}
          title="Next day"
        >
          ▶
        </button>
        ${date !== todayDate &&
        html`<button class="day-nav" style="width:auto;padding:0 8px"
                     onClick=${() => setDate(todayDate)}>today</button>`}
        <label class="smooth-control">
          smoothing
          <input
            type="range"
            min="0"
            max=${SMOOTH_STEPS.length - 1}
            step="1"
            value=${smoothIdx}
            onInput=${(e) => setSmoothIdx(Number(e.target.value))}
          />
          <span class="smooth-value">${`${windowMins} min`}</span>
        </label>
      </div>
      ${err && html`<p class="chart-empty">Failed to load: ${err}</p>`}
      ${!err && !data && html`<p class="chart-empty">Loading…</p>`}
      ${data &&
      html`
        <${DayTimelineChart}
          buckets=${data.buckets}
          bucketSecs=${data.bucket_secs}
          windowMins=${windowMins}
        />
        ${sessions.length > 0 &&
        html`
          <div class="meta-hint">
            ${`${sessions.length} session${sessions.length === 1 ? '' : 's'} · ${Math.round(totalMins)} min read`}
          </div>
        `}
      `}
    </div>
  `;
}

function AnkiCard({ anki, onRefresh, busy }) {
  if (!anki) return null;
  if (!anki.available) {
    return html`
      <div class="card">
        <h2>Anki · mined-word re-encounters</h2>
        <div class="meta-hint">
          No deck snapshot yet — open Anki (desktop or phone) and refresh.
        </div>
        <button class="pause-btn" onClick=${onRefresh} disabled=${busy}>
          ${busy ? "refreshing…" : "↻ refresh from Anki"}
        </button>
      </div>
    `;
  }
  const pct =
    anki.mined > 0 ? ((anki.reencountered / anki.mined) * 100).toFixed(0) : 0;
  const ageMins = Math.round((Date.now() / 1000 - anki.snapshot_ts) / 60);
  // One string, not markup: line breaks between text and ${...} inside an
  // element get collapsed by htm, so a prettier reflow eats the spaces.
  const snapshotAge = `snapshot ${
    ageMins < 60 ? `${ageMins} min` : `${Math.round(ageMins / 60)} h`
  } ago`;
  return html`
    <div class="card">
      <h2>Anki · mined-word re-encounters</h2>
      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">mined words</div>
          <div class="value">${anki.mined.toLocaleString("en")}</div>
        </div>
        <div class="tile">
          <div class="label">re-encountered</div>
          <div class="value">
            ${anki.reencountered.toLocaleString("en")}
            <span class="value-sub">(${pct}%)</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">encounters · 7d</div>
          <div class="value">${anki.week_encounters.toLocaleString("en")}</div>
        </div>
      </div>
      ${anki.top_week.length > 0 &&
      html`
        <div class="word-list-label">most met this week</div>
        <div class="word-chips">
          ${anki.top_week.map(
            (w) => html`<span class="chip">${w.word} <b>×${w.count}</b></span>`,
          )}
        </div>
      `}
      ${anki.never_count > 0 &&
      html`
        <details class="never-seen">
          <summary>
            ${anki.never_count.toLocaleString("en")} mined words not
            re-encountered yet
          </summary>
          <div class="word-chips">
            ${anki.never_sample.map(
              (w) => html`<span class="chip">${w}</span>`,
            )}
            ${anki.never_count > anki.never_sample.length &&
            html`<span class="chip">…</span>`}
          </div>
        </details>
      `}
      <div class="anki-footer">
        <span>${snapshotAge}</span>
        <button class="pause-btn" onClick=${onRefresh} disabled=${busy}>
          ${busy ? "refreshing…" : "↻ refresh"}
        </button>
      </div>
    </div>
  `;
}

const LOOKUP_STATUS = {
  mined: "carded",
  known: "had card",
  unmined: "not carded",
};

function LookupsCard({ lookups }) {
  if (!lookups || lookups.terms === 0) {
    return html`
      <div class="card">
        <h2>Lookups</h2>
        <p class="chart-empty">
          No lookups recorded yet. Point Yomitan's server address at
          /anki-proxy.
        </p>
      </div>
    `;
  }
  const pct = (n) => Math.round((n / lookups.terms) * 100);
  return html`
    <div class="card">
      <h2>Lookups · what they turn into</h2>
      <div class="tile-row" style="margin-top:0">
        <div class="tile">
          <div class="label">words looked up</div>
          <div class="value">
            ${lookups.terms.toLocaleString("en")}
            <span class="value-sub"
              >(${lookups.events.toLocaleString("en")} lookups)</span
            >
          </div>
        </div>
        <div class="tile">
          <div class="label">became cards</div>
          <div class="value">
            ${lookups.mined.toLocaleString("en")}
            <span class="value-sub">(${pct(lookups.mined)}%)</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">already had a card</div>
          <div class="value">
            ${lookups.known.toLocaleString("en")}
            <span class="value-sub">(${pct(lookups.known)}%)</span>
          </div>
        </div>
        <div class="tile">
          <div class="label">repeat lookups</div>
          <div class="value">
            ${lookups.repeat_events.toLocaleString("en")}
            ${lookups.repeat_terms > 0 &&
            html`<span class="value-sub"
              >(${lookups.repeat_terms} words)</span
            >`}
          </div>
        </div>
      </div>
      ${lookups.repeats.length > 0 &&
      html`
        <div class="word-list-label">looked up more than once</div>
        <div class="word-chips">
          ${lookups.repeats.map(
            (r) => html`
              <span class="chip"
                >${r.term} <b>×${r.times}</b>
                <span class="chip-note">${LOOKUP_STATUS[r.status]}</span></span
              >
            `,
          )}
          ${lookups.repeat_terms > lookups.repeats.length &&
          html`<span class="chip">…</span>`}
        </div>
      `}
      ${lookups.leeches.length > 0 &&
      html`
        <details class="never-seen">
          <summary>
            ${lookups.leech_count.toLocaleString("en")} looked up despite
            already having a card
          </summary>
          <div class="word-chips">
            ${lookups.leeches.map(
              (l) => html`
                <span class="chip"
                  >${l.term}
                  <span class="chip-note"
                    >card ${Math.round(l.card_age_days)}d
                    old${l.times > 1 ? ` · ×${l.times}` : ""}</span
                  >
                </span>
              `,
            )}
            ${lookups.leech_count > lookups.leeches.length &&
            html`<span class="chip">…</span>`}
          </div>
        </details>
      `}
      ${lookups.median_mine_secs !== null &&
      html`
        <div class="anki-footer">
          <span
            >median ${Math.round(lookups.median_mine_secs)}s from lookup to
            card</span
          >
        </div>
      `}
    </div>
  `;
}

/* Dialogue vs narration ----------------------------------------------------

   Japanese marks speech with 「」, so the split is already in the raw text and
   costs nothing to derive. Two measures, two categories — and the measures are
   in different units (chars/hour against lookups per 1000 chars), so they get a
   bar group each rather than one plot with two scales. Each group is scaled to
   its own max, which is what makes the *comparison* legible; the absolute
   number sits at the end of every bar so the scaling can't mislead. */

const DIALOGUE_COLOR = "var(--series-1)";
const NARRATION_COLOR = "var(--series-2)";

function CompareBars({ title, unit, rows, format }) {
  const max = Math.max(...rows.map((r) => r.value));
  return html`
    <div class="compare">
      <div class="compare-title">${title}</div>
      ${rows.map(
        (r) => html`
          <div class="compare-row">
            <span class="compare-name">${r.label}</span>
            <span class="compare-track">
              <span
                class="compare-fill"
                style=${`width:${max > 0 ? (r.value / max) * 100 : 0}%;background:${r.color}`}
              ></span>
            </span>
            <span class="compare-value">${format(r.value)}</span>
          </div>
        `,
      )}
      <div class="compare-unit">${unit}</div>
    </div>
  `;
}

/** The comparison in words. Derived rather than written out, so it stays true
    if the two ever swap places — which is the interesting case, not a bug. */
function dialogueVerdict(d, n) {
  if (!d.speed || !n.speed) return null;
  const slower = n.speed < d.speed ? "narration" : "dialogue";
  const faster = slower === "narration" ? "dialogue" : "narration";
  const slowPct = Math.round(
    (1 - Math.min(d.speed, n.speed) / Math.max(d.speed, n.speed)) * 100,
  );
  const speedPart = `${slower} reads ${slowPct}% slower than ${faster}`;

  if (d.lookups_per_1k === null || n.lookups_per_1k === null) {
    return `${speedPart[0].toUpperCase()}${speedPart.slice(1)}.`;
  }
  const denser = n.lookups_per_1k > d.lookups_per_1k ? "narration" : "dialogue";
  const ratio = (
    Math.max(d.lookups_per_1k, n.lookups_per_1k) /
    Math.min(d.lookups_per_1k, n.lookups_per_1k)
  ).toFixed(1);
  const lookupPart = `takes ${ratio}× the lookups per character`;

  // The two measures usually agree — the slower half is the denser one — and
  // when they do the card can say which half is simply harder. When they part
  // company that is the finding, so don't paper over it with one clause.
  return denser === slower
    ? `${slower === "narration" ? "Prose" : "Speech"} is the harder half: ${speedPart} and ${lookupPart}.`
    : `${speedPart[0].toUpperCase()}${speedPart.slice(1)}, yet ${denser} ${lookupPart} — the slower half is not the one with more unknown words.`;
}

function DialogueCard({ dialogue }) {
  if (!dialogue || dialogue.overall.share === null) {
    return html`
      <div class="card">
        <h2>Dialogue vs narration</h2>
        <p class="chart-empty">
          Nothing classified yet — this reads 「」 out of the stored line text,
          so it fills in as the logger captures lines.
        </p>
      </div>
    `;
  }
  const { dialogue: d, narration: n, share } = dialogue.overall;
  const dPct = Math.round(share * 100);
  const today = dialogue.today;

  // Pre-built strings: htm collapses whitespace where literal text meets an
  // ${...} across a line break (see CLAUDE.md).
  const dLabel = `dialogue ${dPct}%`;
  const nLabel = `narration ${100 - dPct}%`;
  const todayLabel =
    today.share === null
      ? "nothing hooked today yet — split on 「」 in the line text"
      : `today ${Math.round(today.share * 100)}% dialogue · split on 「」 in the line text`;

  const verdict = dialogueVerdict(d, n);

  return html`
    <div class="card">
      <h2>Dialogue vs narration</h2>

      <div class="split-bar">
        <span
          class="split-seg"
          style=${`width:${dPct}%;background:${DIALOGUE_COLOR}`}
        ></span>
        <span
          class="split-seg"
          style=${`width:${100 - dPct}%;background:${NARRATION_COLOR}`}
        ></span>
      </div>
      <div class="split-caption">
        <span><span class="legend-swatch" style=${`background:${DIALOGUE_COLOR}`}></span>${dLabel}</span>
        <span><span class="legend-swatch" style=${`background:${NARRATION_COLOR}`}></span>${nLabel}</span>
      </div>
      <div class="meta-hint">${todayLabel}</div>

      <${CompareBars}
        title="reading speed"
        unit="chars/hour, over lines that were wholly one or the other"
        format=${(v) => Math.round(v).toLocaleString("en")}
        rows=${[
          { label: "dialogue", value: d.speed, color: DIALOGUE_COLOR },
          { label: "narration", value: n.speed, color: NARRATION_COLOR },
        ]}
      />

      ${d.lookups_per_1k !== null &&
      n.lookups_per_1k !== null &&
      html`
        <${CompareBars}
          title="unknown-word rate"
          unit="lookups per 1000 characters"
          format=${(v) => v.toFixed(2)}
          rows=${[
            { label: "dialogue", value: d.lookups_per_1k, color: DIALOGUE_COLOR },
            { label: "narration", value: n.lookups_per_1k, color: NARRATION_COLOR },
          ]}
        />
      `}

      ${verdict && html`<p class="chart-note">${verdict}</p>`}
    </div>
  `;
}

function DaysTable({ days, todayDate }) {
  const recent = days.slice(-14).reverse();
  return html`
    <div class="card">
      <h2>Recent days</h2>
      <table class="days">
        <thead>
          <tr>
            <th>date</th>
            <th>time</th>
            <th>chars</th>
            <th>chars/h</th>
            <th>lookups/1k</th>
            <th>focus</th>
          </tr>
        </thead>
        <tbody>
          ${recent.map(
            (d) => html`
              <tr class=${d.date === todayDate ? "today" : ""}>
                <td>${d.date}</td>
                <td>${d.active_secs > 0 ? fmtMins(d.active_secs) : "—"}</td>
                <td>${d.chars > 0 ? d.chars.toLocaleString("en") : "—"}</td>
                <td>
                  ${d.active_secs >= 600
                    ? Math.round(
                        d.chars / (d.active_secs / 3600),
                      ).toLocaleString("en")
                    : "—"}
                </td>
                <td>
                  ${d.lookups_per_1k !== null
                    ? d.lookups_per_1k.toFixed(1)
                    : "—"}
                </td>
                <td
                  title=${d.focus.interruptions > 0
                    ? `${d.focus.interruptions} interruptions`
                    : ""}
                >
                  ${d.focus.ratio !== null
                    ? `${Math.round(d.focus.ratio * 100)}%`
                    : "—"}
                </td>
              </tr>
            `,
          )}
        </tbody>
      </table>
    </div>
  `;
}

function App() {
  const [summary, setSummary] = useState(null);
  const [days, setDays] = useState(null);
  const [works, setWorks] = useState([]);
  const [settings, setSettings] = useState(null);
  const [sessions, setSessions] = useState(null);
  const [anki, setAnki] = useState(null);
  const [lookups, setLookups] = useState(null);
  const [dialogue, setDialogue] = useState(null);
  const [ankiBusy, setAnkiBusy] = useState(false);
  const [error, setError] = useState(null);

  async function load() {
    try {
      const [s, d, w, cfg, sess, ank, lk, dlg] = await Promise.all([
        api("/api/summary"),
        api("/api/days?days=60"),
        api("/api/works"),
        api("/api/settings"),
        api("/api/sessions"),
        api("/api/anki/summary"),
        api("/api/lookups/summary"),
        api("/api/dialogue/summary?days=60"),
      ]);
      setSummary(s);
      setDays(d);
      setWorks(w);
      setSettings(cfg);
      setSessions(sess);
      setAnki(ank);
      setLookups(lk);
      setDialogue(dlg);
      setError(null);
    } catch (err) {
      setError(err.message);
    }
  }

  async function refreshAnki() {
    setAnkiBusy(true);
    try {
      await api("/api/anki/refresh", { method: "POST", body: {} });
    } catch (err) {
      alert(`Anki refresh failed: ${err.message}`);
    } finally {
      setAnkiBusy(false);
      load();
    }
  }

  useEffect(() => {
    load();
    // Best-effort snapshot on open — quietly skipped when no Anki is running.
    api("/api/anki/refresh", { method: "POST", body: {} })
      .then(load)
      .catch(() => {});
    const t = setInterval(load, REFRESH_MS);
    return () => clearInterval(t);
  }, []);

  if (error) return html`<p class="chart-empty">Failed to load: ${error}</p>`;
  if (!summary || !days || !settings)
    return html`<p class="chart-empty">Loading…</p>`;

  async function togglePause() {
    try {
      await api("/api/pause", { method: "POST", body: {} });
      load();
    } catch (err) {
      alert(err.message);
    }
  }

  return html`
    <header>
      <h1>read-stats</h1>
      <div class="header-right">
        <a class="pause-btn" href="#read" title="Live line feed + mine button">
          📖 read
        </a>
        <button
          class="pause-btn ${summary.paused ? "paused" : ""}"
          onClick=${togglePause}
          title="Pause/resume tracking (skipped scenes don't count while paused)"
        >
          ${summary.paused ? "▶ resume tracking" : "⏸ pause"}
        </button>
        <span class="streak"
          >streak <strong>${summary.streak.current}</strong> days
          ${summary.streak.best > summary.streak.current
            ? ` · best ${summary.streak.best}`
            : " · personal best"}</span
        >
      </div>
    </header>
    ${summary.paused &&
    html`<div class="paused-banner">
      Tracking paused — lines are captured but won't count. Resume when you're
      reading again.
    </div>`}
    <${TodayCard} summary=${summary} />
    <${CurrentReading}
      works=${works}
      settings=${settings}
      days=${days}
      onSaved=${load}
    />
    <${WeekTiles} days=${days} />
    <div class="card">
      <h2>Daily minutes · last 30 days</h2>
      <${MinutesBarChart}
        days=${days.slice(-30)}
        floorMins=${summary.goal.floor_mins}
        targetMins=${summary.goal.target_mins}
      />
    </div>
    <div class="card">
      <h2>Reading speed · chars/hour, last 30 days</h2>
      <${SpeedTrendChart} days=${days.slice(-30)} />
    </div>
    <div class="card">
      <h2>Lookups & cards · per hour read, last 30 days</h2>
      <${RateTrendChart} days=${days.slice(-30)} />
    </div>
    <${DayDetailCard} todayDate=${summary.today.date} />
    <${SessionsToday} sessions=${sessions} />
    <${AnkiCard} anki=${anki} onRefresh=${refreshAnki} busy=${ankiBusy} />
    <${LookupsCard} lookups=${lookups} />
    <${DialogueCard} dialogue=${dialogue} />
    <${WorksTable} works=${works} settings=${settings} onSaved=${load} />
    <${LogForm} onLogged=${load} />
    <${DaysTable} days=${days} todayDate=${summary.today.date} />
  `;
}

/** Which of the two views the URL is asking for. The reader is a separate
 *  branch rather than a section of the dashboard so that opening it unmounts
 *  App entirely — no 60s aggregate polling running behind a reading session. */
function useHashRoute() {
  const [hash, setHash] = useState(() => location.hash);
  useEffect(() => {
    const onChange = () => setHash(location.hash);
    addEventListener("hashchange", onChange);
    return () => removeEventListener("hashchange", onChange);
  }, []);
  return hash;
}

function Root() {
  return useHashRoute() === "#read" ? html`<${Reader} />` : html`<${App} />`;
}

render(html`<${Root} />`, document.getElementById("app"));
