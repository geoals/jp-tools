import { render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
import { html } from 'htm/preact';
import { api } from './api.js';
import { GoalMeter, MinutesBarChart, SpeedTrendChart } from './charts.js';

const REFRESH_MS = 60_000;

function fmtMins(secs) {
  const mins = Math.round(secs / 60);
  return mins < 100 ? `${mins} min` : `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, '0')}m`;
}

function fmtChars(n) {
  return n >= 10_000 ? `${(n / 1000).toFixed(1)}k` : n.toLocaleString('en');
}

function TodayCard({ summary }) {
  const { today, goal } = summary;
  const mins = today.active_secs / 60;
  const speed = today.active_secs >= 600 ? today.chars / (today.active_secs / 3600) : null;
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

function NowReading({ settings, onSaved }) {
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  async function submit(e) {
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

  return html`
    <div class="card">
      <form class="now-reading" onSubmit=${submit}>
        <label for="now-reading-input">Now reading (stamped on hooked VN lines)</label>
        <div class="now-reading-row">
          <input id="now-reading-input" name="work" type="text"
                 value=${settings.current_work} placeholder="アイヨクノイウスティア" />
          <button type="submit" disabled=${busy}>${busy ? '…' : 'set'}</button>
          ${saved && html`<span class="form-msg ok">✓</span>`}
        </div>
      </form>
    </div>
  `;
}

function WorksTable({ works }) {
  if (!works.length) return null;
  return html`
    <div class="card">
      <h2>Works</h2>
      <table class="days">
        <thead><tr><th>work</th><th>time</th><th>chars</th><th>last read</th></tr></thead>
        <tbody>
          ${works.slice(0, 10).map((w) => html`
            <tr>
              <td class="work-name">${w.work ?? '(unlabeled)'}</td>
              <td>${fmtMins(w.active_secs)}</td>
              <td>${w.chars.toLocaleString('en')}</td>
              <td>${w.last_read}</td>
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

function DaysTable({ days, todayDate }) {
  const recent = days.slice(-14).reverse();
  return html`
    <div class="card">
      <h2>Recent days</h2>
      <table class="days">
        <thead><tr><th>date</th><th>time</th><th>chars</th><th>chars/h</th></tr></thead>
        <tbody>
          ${recent.map((d) => html`
            <tr class=${d.date === todayDate ? 'today' : ''}>
              <td>${d.date}</td>
              <td>${d.active_secs > 0 ? fmtMins(d.active_secs) : '—'}</td>
              <td>${d.chars > 0 ? d.chars.toLocaleString('en') : '—'}</td>
              <td>${d.active_secs >= 600 ? Math.round(d.chars / (d.active_secs / 3600)).toLocaleString('en') : '—'}</td>
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
  const [error, setError] = useState(null);

  async function load() {
    try {
      const [s, d, w, cfg] = await Promise.all([
        api('/api/summary'),
        api('/api/days?days=60'),
        api('/api/works'),
        api('/api/settings'),
      ]);
      setSummary(s);
      setDays(d);
      setWorks(w);
      setSettings(cfg);
      setError(null);
    } catch (err) {
      setError(err.message);
    }
  }

  useEffect(() => {
    load();
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
    <${NowReading} settings=${settings} onSaved=${load} />
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
    <${WorksTable} works=${works} />
    <${LogForm} onLogged=${load} />
    <${DaysTable} days=${days} todayDate=${summary.today.date} />
  `;
}

render(html`<${App} />`, document.getElementById('app'));
