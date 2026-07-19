import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import { html } from 'htm/preact';
import { api } from './api.js';
import { GoalMeter, MinutesBarChart, ProgressBar, RateTrendChart, SpeedTrendChart } from './charts.js';

const REFRESH_MS = 60_000;

function fmtMins(secs) {
  const mins = Math.round(secs / 60);
  return mins < 100 ? `${mins} min` : `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, '0')}m`;
}

function fmtChars(n) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  return n >= 10_000 ? `${(n / 1000).toFixed(1)}k` : n.toLocaleString('en');
}

/** Mean chars/day over the trailing 7 *complete* days (today excluded — it
 *  would drag the average down all morning), zero days included, clipped to
 *  the pace_start_date cutoff so a reading break doesn't pollute the window.
 *  Falls back to today's partial data when the window is otherwise empty. */
function paceCharsPerDay(days, settings) {
  let win = days.slice(-8, -1);
  if (settings.pace_start_date) {
    win = win.filter((d) => d.date >= settings.pace_start_date);
  }
  if (!win.length) win = days.slice(-1);
  if (!win.length) return 0;
  return win.reduce((a, d) => a + d.chars, 0) / win.length;
}

function fmtFinishDate(daysLeft) {
  const d = new Date(Date.now() + daysLeft * 86_400_000);
  const opts = { month: 'short', day: 'numeric' };
  if (d.getFullYear() !== new Date().getFullYear()) opts.year = 'numeric';
  return `≈ ${d.toLocaleDateString('en', opts)}`;
}

function TodayCard({ summary }) {
  const { today, goal } = summary;
  const mins = today.active_secs / 60;
  const speed = today.active_secs >= 600 ? today.chars / (today.active_secs / 3600) : null;
  // Same 10-minute floor as speed: below it the per-hour denominator is noise.
  const lookupsPerHour = today.active_secs >= 600 ? today.lookups / (today.active_secs / 3600) : null;
  return html`
    <div class="card">
      <h2>Today · ${today.date}</h2>
      <div class="hero-row">
        <span class="hero">${fmtMins(today.active_secs)}</span>
        <span class="hero-sub">
          ${mins >= goal.target_mins
            ? html`<span class="goal-met">target met</span>`
            : mins >= goal.floor_mins
              ? html`<span class="goal-met">floor met</span> · ${Math.ceil(goal.target_mins - mins)} min to target`
              : `${Math.ceil(goal.floor_mins - mins)} min to floor`}
        </span>
      </div>
      <${GoalMeter} mins=${mins} floorMins=${goal.floor_mins} targetMins=${goal.target_mins} />
      <div class="meter-caption"><span>0</span><span>floor ${goal.floor_mins}</span><span>${goal.target_mins} min</span></div>
      <div class="tile-row">
        <div class="tile">
          <div class="label">characters</div>
          <div class="value">${today.chars.toLocaleString('en')}</div>
        </div>
        <div class="tile">
          <div class="label">speed</div>
          <div class="value">${speed ? `${fmtChars(Math.round(speed))}/h` : '—'}</div>
        </div>
        <div class="tile">
          <div class="label">VN / logged</div>
          <div class="value">${fmtChars(today.vn.chars)} / ${fmtChars(today.manual.chars)}</div>
        </div>
        <div class="tile">
          <div class="label">cards mined</div>
          <div class="value">${today.cards > 0 ? today.cards : '—'}</div>
        </div>
        <div class="tile">
          <div class="label">lookups</div>
          <div class="value">
            ${today.lookups > 0 ? today.lookups.toLocaleString('en') : '—'}
            ${lookupsPerHour !== null
              && html`<span class="value-sub">(${lookupsPerHour.toFixed(1)}/h)</span>`}
          </div>
        </div>
        <div class="tile">
          <div class="label">focus</div>
          <div class="value">
            ${today.focus.ratio !== null ? `${Math.round(today.focus.ratio * 100)}%` : '—'}
            ${today.focus.longest_stretch_secs > 0
              && html`<span class="value-sub">(${fmtMins(today.focus.longest_stretch_secs)} best)</span>`}
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
        <div class="tile"><div class="label">time</div><div class="value">${fmtMins(secs)}</div></div>
        <div class="tile"><div class="label">characters</div><div class="value">${chars.toLocaleString('en')}</div></div>
        <div class="tile"><div class="label">days read</div><div class="value">${daysMet}/7</div></div>
        <div class="tile"><div class="label">avg speed</div>
          <div class="value">${secs >= 600 ? `${fmtChars(Math.round(chars / (secs / 3600)))}/h` : '—'}</div></div>
      </div>
    </div>
  `;
}

/** Progress numbers for one work; needs a jpdb total_chars to say anything. */
function workProgress(w, days, settings) {
  const total = w?.meta?.total_chars;
  if (!total) return null;
  const workSpeed = w.active_secs >= 600 ? w.chars / (w.active_secs / 3600) : null;
  const remaining = Math.max(0, total - w.chars);
  const pace = paceCharsPerDay(days, settings);
  return {
    pct: Math.min(100, (w.chars / total) * 100),
    caption: `${fmtChars(w.chars)} / ${fmtChars(total)} chars`,
    remaining,
    hoursLeft: workSpeed ? remaining / workSpeed : null,
    finish: pace > 0 && remaining > 0 ? fmtFinishDate(remaining / pace) : null,
  };
}

function CurrentReading({ works, settings, days, onSaved }) {
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [metaMsg, setMetaMsg] = useState(null);
  const [paceMsg, setPaceMsg] = useState(null);

  const current = settings.current_work
    ? works.find((w) => w.work === settings.current_work)
    : null;
  const meta = current?.meta;
  const prog = workProgress(current, days, settings);

  async function setWork(e) {
    e.preventDefault();
    setBusy(true);
    setSaved(false);
    try {
      await api('/api/settings', {
        method: 'PUT',
        body: { current_work: e.currentTarget.work.value },
      });
      setSaved(true);
      onSaved();
    } catch (err) {
      alert(err.message);
    } finally {
      setBusy(false);
    }
  }

  async function savePace(e) {
    e.preventDefault();
    setPaceMsg(null);
    try {
      await api('/api/settings', {
        method: 'PUT',
        body: { pace_start_date: e.currentTarget.pace.value },
      });
      setPaceMsg({ ok: true, text: 'set ✓' });
      onSaved();
    } catch (err) {
      setPaceMsg({ ok: false, text: err.message });
    }
  }

  async function saveMeta(e) {
    e.preventDefault();
    const f = e.currentTarget;
    const body = {
      title: f.title.value.trim(),
      vndb_id: f.vndb.value.trim() || undefined,
      total_chars: f.total.value ? Number(f.total.value) : undefined,
      status: f.status.value,
    };
    setMetaMsg(null);
    try {
      await api('/api/works', { method: 'POST', body });
      setMetaMsg({ ok: true, text: 'saved ✓' });
      onSaved();
    } catch (err) {
      setMetaMsg({ ok: false, text: err.message });
    }
  }

  return html`
    <div class="card">
      ${current && prog && html`
        <div class="current-work">
          ${meta.cover && html`<img class="cover" src=${meta.cover} alt="cover" />`}
          <div class="info">
            <div class="title">${current.work}</div>
            <${ProgressBar} pct=${prog.pct} label="Progress through ${current.work}" />
            <div class="progress-caption">
              <span>${prog.caption}</span>
              <span>${prog.pct.toFixed(1)}%</span>
            </div>
            <div class="tile-row">
              ${prog.remaining !== null && html`
                <div class="tile"><div class="label">remaining</div>
                  <div class="value">${fmtChars(prog.remaining)}</div></div>
              `}
              <div class="tile"><div class="label">time left</div>
                <div class="value">${prog.hoursLeft !== null ? `${prog.hoursLeft < 10 ? prog.hoursLeft.toFixed(1) : Math.round(prog.hoursLeft)} h` : '—'}</div></div>
              <div class="tile"><div class="label">finish</div>
                <div class="value">${prog.finish ?? '—'}</div></div>
            </div>
          </div>
        </div>
      `}
      ${current && !prog && html`
        <div class="meta-hint">No length for <strong>${current.work}</strong> yet — paste the jpdb character count below (and a VNDB id if you want the cover).</div>
      `}
      <form class="now-reading" onSubmit=${setWork}>
        <label for="now-reading-input">Now reading (stamped on hooked VN lines)</label>
        <div class="now-reading-row">
          <input id="now-reading-input" name="work" type="text"
                 value=${settings.current_work} placeholder="アイヨクノイウスティア" />
          <button type="submit" disabled=${busy}>${busy ? '…' : 'set'}</button>
          ${saved && html`<span class="form-msg ok">✓</span>`}
        </div>
      </form>
      <details class="log meta-edit">
        <summary>Work metadata (jpdb character count · cover) & pace window</summary>
        <form onSubmit=${saveMeta}>
          <div><label>title *</label><input name="title" type="text" required
                 value=${settings.current_work} /></div>
          <div><label>total chars (from jpdb)</label><input name="total" type="number" min="0" /></div>
          <div><label>vndb id or url (cover fetch only)</label><input name="vndb" type="text" placeholder="v3144" /></div>
          <div><label>status</label>
            <select name="status">
              <option value="reading">reading</option>
              <option value="queued">queued</option>
              <option value="finished">finished</option>
              <option value="dropped">dropped</option>
            </select>
          </div>
          <div class="actions">
            <button type="submit">save</button>
            ${metaMsg && html`<span class="form-msg ${metaMsg.ok ? 'ok' : 'error'}">${metaMsg.text}</span>`}
          </div>
        </form>
        <form onSubmit=${savePace}>
          <div><label>pace window start (ignore days before, e.g. after a break)</label>
            <input name="pace" type="date" value=${settings.pace_start_date} /></div>
          <div class="actions">
            <button type="submit">set</button>
            ${paceMsg && html`<span class="form-msg ${paceMsg.ok ? 'ok' : 'error'}">${paceMsg.text}</span>`}
          </div>
        </form>
      </details>
    </div>
  `;
}

function WorksTable({ works }) {
  if (!works.length) return null;
  return html`
    <div class="card">
      <h2>Works</h2>
      <table class="days">
        <thead><tr><th>work</th><th>time</th><th>chars</th><th>progress</th><th>last read</th></tr></thead>
        <tbody>
          ${works.slice(0, 10).map((w) => html`
            <tr>
              <td class="work-name">${w.work ?? '(unlabeled)'}
                ${w.meta && w.meta.status !== 'reading' && html` <span class="status-tag">${w.meta.status}</span>`}</td>
              <td>${w.active_secs > 0 ? fmtMins(w.active_secs) : '—'}</td>
              <td>${w.chars > 0 ? w.chars.toLocaleString('en') : '—'}</td>
              <td>${w.meta?.total_chars
                ? `${Math.min(100, (w.chars / w.meta.total_chars) * 100).toFixed(0)}%`
                : '—'}</td>
              <td>${w.last_read ?? '—'}</td>
            </tr>
          `)}
        </tbody>
      </table>
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
      await api('/api/sessions', { method: 'POST', body });
      setMsg({ ok: true, text: 'logged ✓' });
      f.minutes.value = '';
      f.pages.value = '';
      f.chars.value = '';
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
          <div><label>minutes *</label><input name="minutes" type="number" min="1" required /></div>
          <div><label>pages</label><input name="pages" type="number" min="0" step="0.5" /></div>
          <div><label>chars (overrides pages)</label><input name="chars" type="number" min="0" /></div>
          <div><label>work</label><input name="work" type="text" placeholder="本日は、お日柄もよく" /></div>
          <div><label>source</label>
            <select name="source">
              <option value="book">book</option>
              <option value="manga">manga</option>
              <option value="other">other</option>
            </select>
          </div>
          <div class="actions">
            <button type="submit" disabled=${busy}>${busy ? 'logging…' : 'log'}</button>
            ${msg && html`<span class="form-msg ${msg.ok ? 'ok' : 'error'}">${msg.text}</span>`}
          </div>
        </form>
      </details>
    </div>
  `;
}

function SessionsToday({ sessions }) {
  if (!sessions) return null;
  const rows = [
    ...sessions.derived.map((s) => ({ ...s, kind: 'vn' })),
    ...sessions.manual.map((s) => ({ ...s, kind: s.source, active_secs: s.end_ts - s.start_ts })),
  ].sort((a, b) => a.start_ts - b.start_ts);
  if (!rows.length) return null;
  const hhmm = (ts) => new Date(ts * 1000).toTimeString().slice(0, 5);
  return html`
    <div class="card">
      <h2>Today's sessions</h2>
      <table class="days">
        <thead><tr><th>time</th><th>mins</th><th>chars</th><th>chars/h</th><th>cards</th><th>cards/h</th></tr></thead>
        <tbody>
          ${rows.map((s) => {
            const hours = s.active_secs / 3600;
            return html`
              <tr>
                <td>${hhmm(s.start_ts)}–${hhmm(s.end_ts)} <span class="status-tag">${s.kind}</span></td>
                <td>${Math.round(s.active_secs / 60)}</td>
                <td>${s.chars.toLocaleString('en')}</td>
                <td>${s.active_secs >= 600 ? fmtChars(Math.round(s.chars / hours)) : '—'}</td>
                <td>${s.cards > 0 ? s.cards : '—'}</td>
                <td>${s.cards > 0 && s.active_secs >= 600 ? (s.cards / hours).toFixed(1) : '—'}</td>
              </tr>
            `;
          })}
        </tbody>
      </table>
    </div>
  `;
}

function AnkiCard({ anki, onRefresh, busy }) {
  if (!anki) return null;
  if (!anki.available) {
    return html`
      <div class="card">
        <h2>Anki · mined-word re-encounters</h2>
        <div class="meta-hint">No deck snapshot yet — open Anki (desktop or phone) and refresh.</div>
        <button class="pause-btn" onClick=${onRefresh} disabled=${busy}>
          ${busy ? 'refreshing…' : '↻ refresh from Anki'}
        </button>
      </div>
    `;
  }
  const pct = anki.mined > 0 ? ((anki.reencountered / anki.mined) * 100).toFixed(0) : 0;
  const ageMins = Math.round((Date.now() / 1000 - anki.snapshot_ts) / 60);
  return html`
    <div class="card">
      <h2>Anki · mined-word re-encounters</h2>
      <div class="tile-row" style="margin-top:0">
        <div class="tile"><div class="label">mined words</div>
          <div class="value">${anki.mined.toLocaleString('en')}</div></div>
        <div class="tile"><div class="label">re-encountered</div>
          <div class="value">${anki.reencountered.toLocaleString('en')} <span class="value-sub">(${pct}%)</span></div></div>
        <div class="tile"><div class="label">encounters · 7d</div>
          <div class="value">${anki.week_encounters.toLocaleString('en')}</div></div>
      </div>
      ${anki.top_week.length > 0 && html`
        <div class="word-list-label">most met this week</div>
        <div class="word-chips">
          ${anki.top_week.map((w) => html`<span class="chip">${w.word} <b>×${w.count}</b></span>`)}
        </div>
      `}
      ${anki.never_count > 0 && html`
        <details class="never-seen">
          <summary>${anki.never_count.toLocaleString('en')} mined words not re-encountered yet</summary>
          <div class="word-chips">
            ${anki.never_sample.map((w) => html`<span class="chip">${w}</span>`)}
            ${anki.never_count > anki.never_sample.length && html`<span class="chip">…</span>`}
          </div>
        </details>
      `}
      <div class="anki-footer">
        <span>snapshot ${ageMins < 60 ? `${ageMins} min` : `${Math.round(ageMins / 60)} h`} ago</span>
        <button class="pause-btn" onClick=${onRefresh} disabled=${busy}>
          ${busy ? 'refreshing…' : '↻ refresh'}
        </button>
      </div>
    </div>
  `;
}

const LOOKUP_STATUS = { mined: 'carded', known: 'had card', unmined: 'not carded' };

function LookupsCard({ lookups }) {
  if (!lookups || lookups.terms === 0) {
    return html`
      <div class="card">
        <h2>Lookups · what they turn into</h2>
        <p class="chart-empty">No lookups recorded yet. Point Yomitan's server address at /anki-proxy.</p>
      </div>
    `;
  }
  const pct = (n) => Math.round((n / lookups.terms) * 100);
  return html`
    <div class="card">
      <h2>Lookups · what they turn into</h2>
      <div class="tile-row" style="margin-top:0">
        <div class="tile"><div class="label">words looked up</div>
          <div class="value">${lookups.terms.toLocaleString('en')}
            <span class="value-sub">(${lookups.events.toLocaleString('en')} lookups)</span></div></div>
        <div class="tile"><div class="label">became cards</div>
          <div class="value">${lookups.mined.toLocaleString('en')}
            <span class="value-sub">(${pct(lookups.mined)}%)</span></div></div>
        <div class="tile"><div class="label">already had a card</div>
          <div class="value">${lookups.known.toLocaleString('en')}
            <span class="value-sub">(${pct(lookups.known)}%)</span></div></div>
        <div class="tile"><div class="label">repeat lookups</div>
          <div class="value">${lookups.repeat_events.toLocaleString('en')}
            ${lookups.repeat_terms > 0
              && html`<span class="value-sub">(${lookups.repeat_terms} words)</span>`}</div></div>
      </div>
      ${lookups.repeats.length > 0 && html`
        <div class="word-list-label">looked up more than once</div>
        <div class="word-chips">
          ${lookups.repeats.map((r) => html`
            <span class="chip">${r.term} <b>×${r.times}</b>
              <span class="chip-note">${LOOKUP_STATUS[r.status]}</span></span>
          `)}
          ${lookups.repeat_terms > lookups.repeats.length && html`<span class="chip">…</span>`}
        </div>
      `}
      ${lookups.leeches.length > 0 && html`
        <details class="never-seen">
          <summary>${lookups.leech_count.toLocaleString('en')} looked up despite already having a card</summary>
          <div class="word-chips">
            ${lookups.leeches.map((l) => html`
              <span class="chip">${l.term}
                <span class="chip-note">card ${Math.round(l.card_age_days)}d old${l.times > 1 ? ` · ×${l.times}` : ''}</span>
              </span>
            `)}
            ${lookups.leech_count > lookups.leeches.length && html`<span class="chip">…</span>`}
          </div>
        </details>
      `}
      ${lookups.median_mine_secs !== null && html`
        <div class="anki-footer">
          <span>median ${Math.round(lookups.median_mine_secs)}s from lookup to card</span>
        </div>
      `}
    </div>
  `;
}

function DaysTable({ days, todayDate }) {
  const recent = days.slice(-14).reverse();
  return html`
    <div class="card">
      <h2>Recent days</h2>
      <table class="days">
        <thead><tr><th>date</th><th>time</th><th>chars</th><th>chars/h</th><th>lookups/1k</th><th>focus</th></tr></thead>
        <tbody>
          ${recent.map((d) => html`
            <tr class=${d.date === todayDate ? 'today' : ''}>
              <td>${d.date}</td>
              <td>${d.active_secs > 0 ? fmtMins(d.active_secs) : '—'}</td>
              <td>${d.chars > 0 ? d.chars.toLocaleString('en') : '—'}</td>
              <td>${d.active_secs >= 600 ? Math.round(d.chars / (d.active_secs / 3600)).toLocaleString('en') : '—'}</td>
              <td>${d.lookups_per_1k !== null ? d.lookups_per_1k.toFixed(1) : '—'}</td>
              <td title=${d.focus.interruptions > 0 ? `${d.focus.interruptions} interruptions` : ''}>
                ${d.focus.ratio !== null ? `${Math.round(d.focus.ratio * 100)}%` : '—'}
              </td>
            </tr>
          `)}
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
  const [ankiBusy, setAnkiBusy] = useState(false);
  const [error, setError] = useState(null);

  async function load() {
    try {
      const [s, d, w, cfg, sess, ank, lk] = await Promise.all([
        api('/api/summary'),
        api('/api/days?days=60'),
        api('/api/works'),
        api('/api/settings'),
        api('/api/sessions'),
        api('/api/anki/summary'),
        api('/api/lookups/summary'),
      ]);
      setSummary(s);
      setDays(d);
      setWorks(w);
      setSettings(cfg);
      setSessions(sess);
      setAnki(ank);
      setLookups(lk);
      setError(null);
    } catch (err) {
      setError(err.message);
    }
  }

  async function refreshAnki() {
    setAnkiBusy(true);
    try {
      await api('/api/anki/refresh', { method: 'POST', body: {} });
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
    api('/api/anki/refresh', { method: 'POST', body: {} }).then(load).catch(() => {});
    const t = setInterval(load, REFRESH_MS);
    return () => clearInterval(t);
  }, []);

  if (error) return html`<p class="chart-empty">Failed to load: ${error}</p>`;
  if (!summary || !days || !settings) return html`<p class="chart-empty">Loading…</p>`;

  async function togglePause() {
    try {
      await api('/api/pause', { method: 'POST', body: {} });
      load();
    } catch (err) {
      alert(err.message);
    }
  }

  return html`
    <header>
      <h1>read-stats</h1>
      <div class="header-right">
        <button class="pause-btn ${summary.paused ? 'paused' : ''}" onClick=${togglePause}
                title="Pause/resume tracking (skipped scenes don't count while paused)">
          ${summary.paused ? '▶ resume tracking' : '⏸ pause'}
        </button>
        <span class="streak">streak <strong>${summary.streak.current}</strong> days
          ${summary.streak.best > summary.streak.current ? ` · best ${summary.streak.best}` : ' · personal best'}</span>
      </div>
    </header>
    ${summary.paused && html`<div class="paused-banner">Tracking paused — lines are captured but won't count. Resume when you're reading again.</div>`}
    <${TodayCard} summary=${summary} />
    <${CurrentReading} works=${works} settings=${settings} days=${days} onSaved=${load} />
    <${WeekTiles} days=${days} />
    <div class="card">
      <h2>Daily minutes · last 30 days</h2>
      <${MinutesBarChart} days=${days.slice(-30)}
        floorMins=${summary.goal.floor_mins} targetMins=${summary.goal.target_mins} />
    </div>
    <div class="card">
      <h2>Reading speed · chars/hour, last 60 days</h2>
      <${SpeedTrendChart} days=${days} />
    </div>
    <div class="card">
      <h2>Lookups & cards · per hour read, last 60 days</h2>
      <${RateTrendChart} days=${days} />
    </div>
    <${SessionsToday} sessions=${sessions} />
    <${AnkiCard} anki=${anki} onRefresh=${refreshAnki} busy=${ankiBusy} />
    <${LookupsCard} lookups=${lookups} />
    <${WorksTable} works=${works} />
    <${LogForm} onLogged=${load} />
    <${DaysTable} days=${days} todayDate=${summary.today.date} />
  `;
}

render(html`<${App} />`, document.getElementById('app'));
