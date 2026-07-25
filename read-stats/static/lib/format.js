// Number and date formatting shared across the panels.
//
// One definition each, because the dashboard is read at a glance and two
// panels disagreeing about how to write 12,345 characters or 1h 04m reads as a
// bug in the data.


export function fmtMins(secs) {
  const mins = Math.round(secs / 60);
  return mins < 100
    ? `${mins} min`
    : `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, "0")}m`;
}

/** Hours read, for totals that run to tens or hundreds of them — where
 *  `fmtMins`' "72h 14m" is more precision than the number can carry. */

export function fmtHours(secs) {
  const hours = secs / 3600;
  if (hours < 1) return `${Math.round(secs / 60)} min`;
  return `${hours < 10 ? hours.toFixed(1) : Math.round(hours)} h`;
}

export function fmtChars(n) {
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

export function fmtFinishDate(daysLeft) {
  const d = new Date(Date.now() + daysLeft * 86_400_000);
  const opts = { month: "short", day: "numeric" };
  if (d.getFullYear() !== new Date().getFullYear()) opts.year = "numeric";
  return `≈ ${d.toLocaleDateString("en", opts)}`;
}

/** An ISO date the backend stamped (first_read / last_read), as "Jul 31". */

export function fmtDateStr(iso) {
  if (!iso) return null;
  const d = new Date(`${iso}T00:00`);
  if (Number.isNaN(d.getTime())) return null;
  const opts = { month: "short", day: "numeric" };
  if (d.getFullYear() !== new Date().getFullYear()) opts.year = "numeric";
  return d.toLocaleDateString("en", opts);
}
