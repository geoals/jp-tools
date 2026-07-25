// Derived reading figures the server does not compute: where the current pace
// lands a work, and what a day's hours imply for a finish date.
//
// These live on the client because they all depend on a *projection* — how much
// you would read from here on — which is a question about intent, not history,
// and the answer changes as you drag the controls.

import { fmtChars, fmtDateStr, fmtFinishDate } from "../lib/format.js";

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

/** Mean active *hours* per calendar day over the same trailing window
 *  `paceCharsPerDay` uses, zero days included. This is a property of the
 *  reader — how much time you sit down for — not of any one VN, so a finish
 *  estimate multiplies it by the current work's own speed rather than by a
 *  cross-work chars/day that a faster previous VN inflated. */

function dailyActiveHours(days, settings) {
  let win = days.slice(-8, -1);
  if (settings.pace_start_date) {
    win = win.filter((d) => d.date >= settings.pace_start_date);
  }
  if (!win.length) win = days.slice(-1);
  if (!win.length) return 0;
  return win.reduce((a, d) => a + d.active_secs, 0) / 3600 / win.length;
}

/** This work's own reading speed in chars/hour, or null below a 10-minute
 *  floor where the denominator is noise. This is the number that halves when
 *  you switch to a harder VN — kept per-work so nothing averages it against
 *  another title. */

export function workSpeedPerHour(w) {
  return w.active_secs >= 600 ? w.chars / (w.active_secs / 3600) : null;
}

/** Progress numbers for one work; needs a jpdb total_chars to say anything. */

export function workProgress(w, days, settings) {
  const total = w?.meta?.total_chars;
  if (!total) return null;
  const workSpeed = workSpeedPerHour(w);
  const remaining = Math.max(0, total - w.chars);
  const { pace, days: paceDays, partial } = paceCharsPerDay(days, settings);
  const window = partial
    ? "today so far (no complete day in the window yet)"
    : `the last ${paceDays} complete day${paceDays === 1 ? "" : "s"}, today excluded`;
  const cutoff = settings.pace_start_date
    ? ` Days before ${settings.pace_start_date} are excluded.`
    : "";

  // Finish estimate, decomposed. How many hours a day you read is a property of
  // you and holds across VNs; how fast you read is a property of *this* VN and
  // does not. So project with (this work's speed × your daily hours) rather
  // than a cross-work chars/day, which would let a faster previous VN pull a
  // harder current one's date earlier than it should. Falls back to the old
  // cross-work pace until the work has 10 minutes of its own to measure speed.
  const dailyHours = dailyActiveHours(days, settings);
  const workCharsPerDay =
    workSpeed !== null && dailyHours > 0 ? workSpeed * dailyHours : null;
  const finishPace = workCharsPerDay ?? (pace > 0 ? pace : null);
  const finishHint =
    finishPace !== null && remaining > 0
      ? workCharsPerDay !== null
        ? `${fmtChars(remaining)} chars left ÷ ${fmtChars(Math.round(workCharsPerDay))} chars/day — ` +
          `this work's own ${fmtChars(Math.round(workSpeed))}/h × ${dailyHours.toFixed(1)} h/day, ` +
          `your average over ${window} (zero days counted).${cutoff}`
        : `${fmtChars(remaining)} chars left ÷ ${fmtChars(Math.round(pace))} chars/day, ` +
          `your cross-work average over ${window} — this work has under 10 minutes of its own to gauge speed.${cutoff}`
      : null;

  // A finished work reports its real dates rather than a projection: the
  // estimate is meaningless once there's nothing left to read.
  const done = w?.meta?.status === "finished";
  return {
    pct: Math.min(100, (w.chars / total) * 100),
    caption: `${fmtChars(w.chars)} / ${fmtChars(total)} chars`,
    remaining,
    speed: workSpeed,
    hoursLeft: workSpeed ? remaining / workSpeed : null,
    finish:
      finishPace !== null && remaining > 0
        ? fmtFinishDate(remaining / finishPace)
        : null,
    finishHint,
    done,
    started: fmtDateStr(w.first_read),
    finished: done ? fmtDateStr(w.last_read) : null,
  };
}
