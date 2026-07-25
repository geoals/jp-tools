// Light/dark, and the one place that decides it.
//
// The palette in css/base.css already answers to both the OS signal and an
// explicit `data-theme` stamp on <html>. All this does is set the stamp, and
// remember the choice — so a preference survives a reload, and "system" really
// means system rather than a frozen copy of what the OS said at first paint.
//
// Applied before render (see spa.html) so there is no light flash on a dark
// device: the stamp has to be on the element before the first paint, which a
// component effect is too late for.

const KEY = "read-stats-theme";

/** The three states the control offers. "system" removes the stamp entirely. */
export const THEMES = ["system", "light", "dark"];

export function storedTheme() {
  const v = localStorage.getItem(KEY);
  return THEMES.includes(v) ? v : "system";
}

export function applyTheme(theme) {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
}

export function setTheme(theme) {
  if (!THEMES.includes(theme)) return;
  localStorage.setItem(KEY, theme);
  applyTheme(theme);
}
