// Light/dark, and the one place that decides it.
//
// The palette in css/base.css answers to both the OS signal and an explicit
// `data-theme` stamp on <html>; this sets the stamp and remembers the choice.
// "system" removes the stamp, so it keeps tracking the OS as it changes.
//
// Applied before render (see spa.html) so there is no light flash on a dark
// device: the stamp has to be on the element before the first paint, which a
// component effect is too late for.

const KEY = "kotodex-theme";

/** The three states the control offers. */
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
