// The dashboard's entry point: routing, the data every panel shares, and the
// page layout.
//
// Everything that draws a card lives in ./panels; everything that draws an SVG
// lives in ./charts.js. What stays here is what genuinely belongs to the whole
// page — the single poll that feeds all of it, and which of the two views
// (dashboard or #read) is on screen.
//
// Loading it once and passing it down, rather than each panel fetching its own,
// is deliberate: half the cards are different readings of the same days, and
// letting them fetch independently would show a stale streak beside a fresh
// chart for as long as the requests disagreed.

import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { html } from "htm/preact";
import { api } from "./api.js";
import { Reader } from "./reader.js";
import { RateTrendChart, SpeedTrendChart } from "./charts.js";
import { AnkiCard } from "./panels/anki.js";
import { CurrentReading } from "./panels/current-reading.js";
import { DailyChartCard, WeekTiles } from "./panels/daily.js";
import { DayDetailCard } from "./panels/day-detail.js";
import { DaysTable } from "./panels/days-table.js";
import { DialogueCard } from "./panels/dialogue.js";
import { LogForm } from "./panels/log-form.js";
import { LookupsCard } from "./panels/lookups.js";
import { SessionsToday } from "./panels/sessions.js";
import { TodayCard } from "./panels/today.js";
import { WorksTable } from "./panels/works-table.js";

const REFRESH_MS = 60_000;

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
          ${
            summary.streak.best > summary.streak.current
              ? ` · best ${summary.streak.best}`
              : " · personal best"
          }</span
        >
      </div>
    </header>
    ${
      summary.paused &&
      html`<div class="paused-banner">
        Tracking paused — lines are captured but won't count. Resume when you're
        reading again.
      </div>`
    }
    <${TodayCard} summary=${summary} />
    <${CurrentReading}
      works=${works}
      settings=${settings}
      days=${days}
      onSaved=${load}
    />
    <${WeekTiles} days=${days} />
    <${DailyChartCard}
      days=${days}
      dialogue=${dialogue}
      floorMins=${summary.goal.floor_mins}
      targetMins=${summary.goal.target_mins}
    />
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
    <${DialogueCard}
      dialogue=${dialogue}
      currentWork=${settings.current_work}
    />
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

render(html`<${Root} />`, document.getElementById("app"));
