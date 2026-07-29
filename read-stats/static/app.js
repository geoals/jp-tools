// The dashboard's entry point: routing, the data every panel shares, and the
// page layout.
//
// Everything that draws a card lives in ./panels; everything that draws an SVG
// lives in ./charts.js. What stays here is what genuinely belongs to the whole
// page — the single poll that feeds all of it, and which view is on screen.
//
// Loading it once and passing it down, rather than each panel fetching its own,
// is deliberate: half the cards are different readings of the same days, and
// letting them fetch independently would show a stale streak beside a fresh
// chart for as long as the requests disagreed. **The tabs choose what renders,
// never what is fetched** — switching tabs must not be able to show two panels
// disagreeing about the same day.
//
// The page used to be twelve cards in one column: today and the last 30 days
// interleaved, four slices of the same window each with its own hardcoded
// range, and an action card in the middle of the statistics. It is three tabs
// now, one per question — how is today going, how is it trending, what am I
// reading — and the merges live in ./panels/day.js, trends.js and library.js.

import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { html } from "htm/preact";
import { api } from "./api.js";
import { fmtTsDate } from "./lib/format.js";
import { Reader } from "./reader.js";
import { CurrentReading } from "./panels/current-reading.js";
import { DayCard } from "./panels/day.js";
import { LibraryView } from "./panels/library.js";
import { SettingsView } from "./panels/settings.js";
import { KanjiView } from "./panels/kanji.js";
import { VocabView } from "./panels/vocab.js";
import { TrendsCard } from "./panels/trends.js";

const REFRESH_MS = 60_000;

/** The tabs, and the hash each answers to. Order is the order of the question:
 *  what is happening now, what it adds up to, what produced it. */

const TABS = [
  { id: "today", label: "Today" },
  { id: "trends", label: "Trends" },
  { id: "library", label: "Library" },
  { id: "kanji", label: "Kanji" },
  { id: "vocab", label: "Vocab" },
];

function App({ view, sub }) {
  const [summary, setSummary] = useState(null);
  const [days, setDays] = useState(null);
  const [works, setWorks] = useState([]);
  const [settings, setSettings] = useState(null);
  const [sessions, setSessions] = useState(null);
  const [anki, setAnki] = useState(null);
  const [lookups, setLookups] = useState(null);
  const [dialogue, setDialogue] = useState(null);
  const [kanji, setKanji] = useState(null);
  const [vocab, setVocab] = useState(null);
  const [ankiBusy, setAnkiBusy] = useState(false);
  const [error, setError] = useState(null);

  async function load() {
    try {
      const [s, d, w, cfg, sess, ank, lk, dlg, kj, vc] = await Promise.all([
        api("/api/summary"),
        api("/api/days?days=60"),
        api("/api/works"),
        api("/api/settings"),
        api("/api/sessions"),
        api("/api/anki/summary"),
        api("/api/lookups/summary"),
        api("/api/dialogue/summary?days=60"),
        api("/api/kanji"),
        api("/api/vocab/summary"),
      ]);
      setSummary(s);
      setDays(d);
      setWorks(w);
      setSettings(cfg);
      setSessions(sess);
      setAnki(ank);
      setLookups(lk);
      setDialogue(dlg);
      setKanji(kj);
      setVocab(vc);
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

  // Capture, not accounting: this closes the logger's Textractor connection,
  // so nothing enters the line stream while it is off. Nothing is filtered
  // after the fact any more, which is why the banner says "not being captured"
  // rather than "won't count".
  async function togglePause() {
    try {
      await api("/api/capture/pause", { method: "POST", body: {} });
      load();
    } catch (err) {
      alert(err.message);
    }
  }

  const isSettings = view === "settings";
  const tab = TABS.some((t) => t.id === view) ? view : "today";

  // The periodic sweep's trigger is "after a day of reading", which is a thing
  // you notice here and not on the vocab tab. One line, only when there is a
  // batch waiting, and never a number the sweep itself would disagree with —
  // `ready_since` is the same count `/api/vocab/queue` would return.
  const sweepReady = vocab?.ready_since ?? 0;
  const sweptOn = fmtTsDate(vocab?.swept_through);
  const sweepLine =
    sweepReady > 0
      ? `${sweepReady.toLocaleString("en")} words ready to judge${sweptOn ? ` since ${sweptOn}` : ""}`
      : null;

  return html`
    <header>
      <h1>read-stats</h1>
      <nav class="tabs">
        ${TABS.map(
          (t) => html`
            <a
              key=${t.id}
              class=${`tab${!isSettings && tab === t.id ? " tab-on" : ""}`}
              href=${`#${t.id}`}
              aria-current=${!isSettings && tab === t.id ? "page" : null}
              >${t.label}</a
            >
          `,
        )}
      </nav>
      <div class="header-right">
        <a
          class="pause-btn"
          href="#read"
          title="Live line feed + explain button"
        >
          📖 read
        </a>
        <a
          class=${`pause-btn${isSettings ? " paused" : ""}`}
          href=${isSettings ? "#today" : "#settings"}
          title="Goal, thresholds and theme"
        >
          ⚙
        </a>
        <button
          class="pause-btn ${summary.paused ? "paused" : ""}"
          onClick=${togglePause}
          title="Stop capture at the source — the logger closes its Textractor connection, so no lines are recorded at all"
        >
          ${summary.paused ? "▶ resume capture" : "⏸ pause capture"}
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
        Capture paused — the logger is disconnected from Textractor and no lines
        are being recorded. Resume when you're reading again.
      </div>`
    }
    ${
      !isSettings &&
      tab === "today" &&
      sweepLine &&
      html`<div class="sweep-nudge">
        <span>${sweepLine}</span>
        <a href="#vocab">sweep them →</a>
      </div>`
    }
    ${
      isSettings
        ? html`<${SettingsView} settings=${settings} onSaved=${load} />`
        : tab === "trends"
          ? html`<${TrendsCard}
              days=${days}
              dialogue=${dialogue}
              targetMins=${summary.goal.target_mins}
              todayDate=${summary.today.date}
            />`
          : tab === "kanji"
            ? html`<${KanjiView} kanji=${kanji} />`
            : tab === "vocab"
              ? html`<${VocabView}
                  vocab=${vocab}
                  settings=${settings}
                  onJudged=${load}
                />`
              : tab === "library"
                ? html`<${LibraryView}
                    works=${works}
                    settings=${settings}
                    anki=${anki}
                    lookups=${lookups}
                    dialogue=${dialogue}
                    openWork=${sub}
                    onRefreshAnki=${refreshAnki}
                    ankiBusy=${ankiBusy}
                    onSaved=${load}
                  />`
                : html`
                    <${CurrentReading}
                      works=${works}
                      settings=${settings}
                      days=${days}
                      onSaved=${load}
                    />
                    <${DayCard}
                      days=${days}
                      todayDate=${summary.today.date}
                      goal=${summary.goal}
                    />
                  `
    }
  `;
}

/** Which view the URL is asking for. The reader is a separate branch rather
 *  than a section of the dashboard so that opening it unmounts App entirely —
 *  no 60s aggregate polling running behind a reading session.
 *
 *  A tab may carry one segment after it (`#library/<title>`), so that opening
 *  a work is a real navigation: back returns to the shelf, and the page can be
 *  linked and reloaded onto the work you were looking at. State held in a
 *  `useState` cannot do either — back would leave the tab entirely. */

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
  const hash = useHashRoute();
  if (hash === "#read") return html`<${Reader} />`;
  // Titles are percent-encoded, and encodeURIComponent escapes "/" too, so
  // splitting on it can never cut a title in half.
  const [view, ...rest] = hash.replace(/^#/, "").split("/");
  const sub = rest.length ? decodeURIComponent(rest.join("/")) : null;
  return html`<${App} view=${view || "today"} sub=${sub} />`;
}

render(html`<${Root} />`, document.getElementById("app"));
