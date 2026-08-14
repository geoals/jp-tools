// The overlay strip's whole client: draw the newest line, and define the word
// that is clicked in it.
//
// `backlog` is what the stream's query parameter exists for — this caller wants
// a short feed, not the whole sitting. Only the newest line is ever drawn; the
// few before it are asked for so the explain button has context to send from
// the moment the overlay opens. On a dropped connection EventSource reconnects
// with `Last-Event-ID`, so it resumes after the line it drew rather than
// replaying anything.
//
// Segmentation is not asked for separately: the line event already carries a
// span per word, each with the `(headword, reading)` the ledger keys on. The
// popup asks about that pair, so 振っ is defined as 振る. Where the tokenizer
// got the position wrong — 経年劣化 split into 経年 and 劣化, 素振り read as
// すぶり where the line means そぶり — the popup carries a chip per other match
// the dictionaries offer, and picking one re-opens it on that term; see
// expansions().
//
// Three actions on a word, and only one of them opens the popup: left-click
// asks what it means, the back side button judges it known or unknown, the
// forward one mines it. Splitting them that way is what keeps the lookup count
// honest — see `SIDE_ACTIONS`. The wheel over that word pages the popup's
// dictionaries, which is not a fourth action: the popup is already open, so
// nothing is looked up and nothing is written.

import { createPopup } from "/shared/popup.js";
import { parseMarkdown } from "/shared/markdown.js";
import { streamExplain } from "/shared/explain.js";

const params = new URLSearchParams(location.search);
const root = document.documentElement.style;
root.setProperty("--strip", `${params.get("h") ?? "300"}px`);
const scale = params.get("scale") ?? "1";
root.setProperty("--scale", scale);
// `mobile=1` — the overlay read off a phone. The line is not being fitted over
// the game's own text any more, so it sits on the bottom edge instead; see
// #box's rules.
const mobile = params.get("mobile") === "1";
if (mobile) document.documentElement.dataset.mobile = "";
// Only the line, never the popup: the popup is a dictionary, and reading it in
// a display face the game text is being tried in makes both harder to judge.
const font = params.get("font");
if (font) root.setProperty("--line-font", `"${font}", sans-serif`);

const lineEl = document.getElementById("line");
const warnEl = document.getElementById("warn");
const popupEl = document.getElementById("popup");

// The popup itself is `web-shared/popup.js`, the same module yt-mine loads —
// what a word means does not depend on which surface asked. What stays here is
// everything that is about *this* surface: where the popup sits over a
// layer-shell strip, the lookup the popup's opening records, and the side
// mouse buttons that judge and mine without opening it at all.
const popup = createPopup({
  el: popupEl,
  api: {
    define: (query) => `/api/reader/define?${query}`,
    expand: (text) => `/api/reader/expand?${new URLSearchParams({ text })}`,
    mined: (term) => `/api/reader/mined?term=${encodeURIComponent(term)}`,
    browse: (note_id) =>
      fetch("/api/reader/mined/browse", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ note_id }),
      }).catch(() => {}),
  },
  scanText: (target) => (line ? line.text.slice(target.start) : ""),
  judge: (target, status) => judge(popup.anchor(), status, target),
  mine: (target) => mine(popup.anchor(), target),
  place,
  onOpen: (data) => (openLookup = data.lookup_id ?? null),
  onJudged,
  onLayout: () => report(),
});

let line = null;
// The overlay shell, once its channel is up. Null in an ordinary browser.
let shell = null;
// The game window's rectangle as the shell last found it, or null when it has
// none to give. Non-null is what puts the line over the game's own text.
let game = null;
let windowName = "";
// The lookup row the open popup was recorded as, so marking the word known can
// take it back. Cleared with the popup: a retraction is only ever the popup
// that made the row undoing itself.
let openLookup = null;
// The rank at or under which an unknown word is called common. Fetched once;
// the same setting the reading view underlines by, so both agree.
let commonMaxRank = 0;
fetch("/api/settings")
  .then((r) => r.json())
  .then((s) => (commonMaxRank = s.reader_common_max_freq_rank || 0))
  .catch(() => {});

// Lines sent to the model with the one to explain, matching `#read`. Also the
// backlog asked for, so an overlay opened mid-scene can explain the first line
// it draws — only the newest is ever shown, the rest exist to place a pronoun.
const EXPLAIN_CONTEXT_LINES = 8;
// The last few lines drawn, oldest first. The stream sends one at a time and
// the explain endpoint wants the run they arrived in.
const recent = [];

const stream = new EventSource(`/api/lines/stream?backlog=${EXPLAIN_CONTEXT_LINES}`);

stream.onmessage = (e) => draw(JSON.parse(e.data));

stream.addEventListener("status", (e) => {
  const { capture, vn_window } = JSON.parse(e.data);
  warnEl.textContent = capture === "live" ? "" : capture;
  // Only the two states that say what the flag is. `down` and `stalled` are
  // faults in the logger, and neither means capture was switched off.
  if (capture === "paused" || capture === "live") showPaused(capture === "paused");
  // Kept even before the channel is up: the first status usually beats it, and
  // the shell is told on connect. The name is per work, so it changes under a
  // running overlay whenever the current work does.
  windowName = vn_window ?? "";
  shell?.setWindowName(windowName);
});

/* Append `text[from..to)` to `parent`, drawing the game's own furigana where it
 * falls. `ruby` offsets are UTF-16 code units over the line, the same units the
 * token spans use, so both index the string directly.
 *
 * The reading is markup here and nowhere else: it is not in `line.text`, so it
 * is not counted, not tokenized, and never reaches a card. Annotations that
 * reach past `to` are left to `draw`, which wraps whole pieces of the line. */
function appendText(parent, text, ruby, from, to) {
  let at = from;
  for (const [start, len, reading] of ruby) {
    const end = start + len;
    if (end <= at || start >= to || start < at || end > to) continue;
    if (start > at) parent.append(drawn(text.slice(at, start)));
    const annotated = document.createElement("ruby");
    annotated.append(drawn(text.slice(start, end)));
    const rt = document.createElement("rt");
    rt.textContent = reading;
    annotated.append(rt);
    parent.append(annotated);
    at = end;
  }
  if (at < to) parent.append(drawn(text.slice(at, to)));
}

/* The game's soft breaks are drawn where the game put them, so the overlaid
 * line keeps the shape it has on screen. At phone size it is not over the game
 * any more and the width is different, so they only break the line in the
 * wrong places — drop them and let it wrap. Only the drawn text changes: the
 * offsets everything else indexes with are over the original string. */
function drawn(slice) {
  return mobile ? slice.replace(/\n/g, "") : slice;
}

// The brackets a line of speech opens with, in the shapes the VNs use them.
const QUOTE_OPEN = /^[「『（(【〈《"]/;

/* The line cut into what is drawn: one piece per word the tokenizer found, and
 * one for each run of text between them. */
function pieces(text, spans) {
  const out = [];
  let at = 0;
  for (const span of spans) {
    if (span.start < at) continue;
    if (span.start > at) out.push({ start: at, end: span.start });
    out.push({ start: span.start, end: span.start + span.len, span });
    at = span.start + span.len;
  }
  if (at < text.length) out.push({ start: at, end: text.length });
  return out;
}

/* The readings that cover more than one piece. The game's markup is the source
 * of truth for what a reading annotates — 牛乳粥 is one word to the game and
 * 牛 + 乳粥 to the tokenizer — so the annotation is drawn whole, with the word
 * spans nested inside the <ruby>. They stay separate elements, so what the
 * reader mines with is untouched.
 *
 * Only when both ends land on a piece boundary. A reading over half a token
 * has nothing to wrap, and appendText draws that stretch as plain text. */
function wideRuby(ruby, parts) {
  const edges = new Set(parts.flatMap((p) => [p.start, p.end]));
  return ruby.filter(([start, len]) => {
    const end = start + len;
    const inOnePiece = parts.some((p) => p.start <= start && end <= p.end);
    return !inOnePiece && edges.has(start) && edges.has(end);
  });
}

/** One line, one span per word the tokenizer found. */
function draw(incoming) {
  closePopup();
  line = incoming;
  recent.push(incoming.text);
  if (recent.length > EXPLAIN_CONTEXT_LINES) recent.shift();
  selectedInLine = "";
  const text = line.text;
  const ruby = line.ruby ?? [];
  const frag = document.createDocumentFragment();
  // Offsets are UTF-16 code units, which is exactly what a JS string indexes
  // in, so they slice directly.
  const parts = pieces(text, [...(line.tokens ?? [])].sort((a, b) => a.start - b.start));
  const wide = wideRuby(ruby, parts);
  let group = null;

  for (const part of parts) {
    const opening = wide.find(([start]) => start === part.start);
    if (opening) group = { end: opening[0] + opening[1], reading: opening[2], el: document.createElement("ruby") };
    const parent = group ? group.el : frag;

    if (!part.span) {
      appendText(parent, text, ruby, part.start, part.end);
    } else {
      const span = part.span;
      const word = document.createElement("span");
      // `known` gets no class, so it draws as plain text.
      const common =
        commonMaxRank &&
        span.freq_rank &&
        span.freq_rank <= commonMaxRank &&
        (span.status === "new" || span.status === "unknown");
      word.className = ["w", span.status === "known" ? "" : span.status, common ? "common" : ""]
        .filter(Boolean)
        .join(" ");
      // The surface travels in a dataset field, not as textContent: a furigana
      // annotation inside the span would otherwise read back as 大事おおごと,
      // which is not a spelling anything is written in and is what the popup,
      // the card and CompactDef would be given.
      word.dataset.surface = text.slice(part.start, part.end);
      appendText(word, text, ruby, part.start, part.end);
      word.dataset.term = span.headword;
      word.dataset.reading = span.reading ?? "";
      word.dataset.status = span.status;
      // Where the word starts in the line, for the expansion scan: it reads the
      // raw text to the right of this point, which the spans do not carry.
      word.dataset.start = String(part.start);
      parent.append(word);
    }

    if (group && part.end === group.end) {
      const rt = document.createElement("rt");
      rt.textContent = group.reading;
      group.el.append(rt);
      frag.append(group.el);
      group = null;
    }
  }

  lineEl.replaceChildren(frag);
  // The game indents a quoted line's later rows under its first character, not
  // under the 「 — see #line.quoted. Reading it off the text rather than always
  // hanging the indent: a narration line starts at the margin and every row of
  // it does.
  lineEl.classList.toggle("quoted", QUOTE_OPEN.test(text));
  report();
}

lineEl.addEventListener("click", (e) => {
  const word = e.target.closest(".w");
  if (!word) return;
  e.stopPropagation();
  // A second click on the same word closes it, so one finger both opens and
  // dismisses without reaching anywhere else.
  if (word === popup.anchor()) return closePopup();
  const previous = popup.anchor();
  if (previous) previous.classList.remove("open");
  word.classList.add("open");
  popup.show(word, {
    term: word.dataset.term,
    key: word.dataset.term,
    reading: word.dataset.reading,
    surface: word.dataset.surface,
    status: word.dataset.status,
    start: Number(word.dataset.start),
  });
});

// Anywhere else on the surface dismisses. Not the popup itself, or scrolling a
// long Jitendex entry would close what is being read.
//
// The popup stops its own clicks rather than the handler below testing where
// they came from. It used to test, and `closest("#popup")` answers about where
// the target is *now*: picking another match re-renders the popup from inside
// the click, which detaches the chip mid-dispatch, and the detached chip then
// read as a click outside — so every pick closed the popup it had just opened.
document.addEventListener("click", () => closePopup());

/** The side buttons act on the word under the pointer, without a popup.
 *
 * Opening the popup is what a lookup *is* — it is the reader asking what a word
 * means, and it is recorded as one. Judging a word already understood, or
 * mining one, is not that, and going through the popup to reach a button
 * recorded a lookup that never happened. So those two moved off the popup
 * entirely and onto the buttons already under the thumb.
 *
 * Back is `button` 3 and forward 4. Both navigate in Chromium, so the default
 * has to go — on `mousedown`, which is where that navigation is armed.
 */
const SIDE_ACTIONS = {
  3: (word) => judge(word, word.dataset.status === "known" ? "unknown" : "known"),
  4: (word) => mine(word),
};

lineEl.addEventListener("mousedown", (e) => {
  if (SIDE_ACTIONS[e.button]) e.preventDefault();
});

lineEl.addEventListener("auxclick", (e) => {
  const action = SIDE_ACTIONS[e.button];
  const word = e.target.closest(".w");
  if (!action || !word) return;
  e.preventDefault();
  action(word);
});

// The wheel over the word the popup is open on pages its dictionaries — the
// hand is already there, having just clicked it. Only over that word: anywhere
// else the wheel still scrolls a line too long for the strip.
lineEl.addEventListener(
  "wheel",
  (e) => {
    if (!popup.isOpen() || e.target.closest(".w") !== popup.anchor()) return;
    e.preventDefault();
    popup.step(Math.sign(e.deltaY));
  },
  { passive: false },
);

// Dragging the strip. Anywhere on the backdrop that is not a word: the words
// are the only thing on it with an action of their own.
//
// It moves the box inside the surface rather than the window — the surface is
// layer-shell, anchored to all four screen edges, and has no position to set.
// The input region follows because it is measured off the box.
const boxEl = document.getElementById("box");
// Per layout: the two start from different places in the strip, so one stored
// drag would carry the mobile line off the bottom edge it is pinned to.
const PLACE = `vn-overlay-offset${mobile ? "-mobile" : ""}`;
// Aligned to the game, the drag is calibration rather than placement — it is
// what the per-game `--text-*` measurements are found with — so it is stored
// apart from the free-floating one, and as fractions of the game window: a
// resize has to carry the correction with the thing it corrects.
const ALIGNED_PLACE = "vn-overlay-offset-aligned";
let drag = null;
let offset = { x: 0, y: 0 };
let alignedOffset = { x: 0, y: 0 };

for (const [key, into] of [[PLACE, "free"], [ALIGNED_PLACE, "aligned"]]) {
  try {
    const stored = JSON.parse(localStorage.getItem(key) ?? "{}");
    if (into === "free") offset = { ...offset, ...stored };
    else alignedOffset = { ...alignedOffset, ...stored };
  } catch {
    // Nothing stored, or stored by an older shape. Start where the CSS puts it.
  }
}
apply();

/** The drag in pixels, whichever of the two is in force. */
function offsetPx() {
  return game
    ? { x: alignedOffset.x * game.w, y: alignedOffset.y * game.h }
    : offset;
}

/** Move to `x, y`, clamped so the box stays somewhere it can be grabbed back
 * from — the surface is the whole screen, and a strip pushed off it is gone. */
function moveTo(x, y) {
  const rect = lineEl.getBoundingClientRect();
  const at = offsetPx();
  const left = rect.left - at.x;
  const top = rect.top - at.y;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  const px = {
    x: clamp(x, -left, Math.max(0, window.innerWidth - rect.width) - left),
    y: clamp(y, -top, Math.max(0, window.innerHeight - rect.height) - top),
  };
  if (game) alignedOffset = { x: px.x / game.w, y: px.y / game.h };
  else offset = px;
  apply();
}

function apply() {
  const at = offsetPx();
  boxEl.style.setProperty("--dx", `${at.x}px`);
  boxEl.style.setProperty("--dy", `${at.y}px`);
  report();
}

lineEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || e.target.closest(".w")) return;
  drag = { id: e.pointerId, x: e.clientX - offset.x, y: e.clientY - offset.y, moved: false };
  lineEl.setPointerCapture(e.pointerId);
});

lineEl.addEventListener("pointermove", (e) => {
  if (!drag || e.pointerId !== drag.id) return;
  drag.moved = true;
  moveTo(e.clientX - drag.x, e.clientY - drag.y);
  // The popup is placed off the line box, so it has to be re-placed as the box
  // moves out from under it.
  if (popup.anchor()) place(popup.anchor());
});

for (const type of ["pointerup", "pointercancel"]) {
  lineEl.addEventListener(type, (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    lineEl.releasePointerCapture(e.pointerId);
    // The click that ends a drag is not a click on the overlay — without this
    // it reaches the document handler and closes the open popup.
    if (drag.moved) document.addEventListener("click", (c) => c.stopPropagation(), { capture: true, once: true });
    drag = null;
    if (game) localStorage.setItem(ALIGNED_PLACE, JSON.stringify(alignedOffset));
    else localStorage.setItem(PLACE, JSON.stringify(offset));
  });
}

// "What does this line say" — the same `/api/reader/explain` `#read` asks, and
// the same `web-shared/markdown.js` over what comes back. Only the surface is
// this file's: a button placed and dragged on its own, because the line box is
// fitted over the game's own text and this has to be able to sit clear of it.
const explainBoxEl = document.getElementById("explain-box");
const explainBtnEl = document.getElementById("explain-btn");
const explainPanelEl = document.getElementById("explain-panel");
// Versioned: the widget used to hang off the bottom edge, and an offset stored
// against that anchor puts it somewhere else entirely against this one.
const EXPLAIN_PLACE = "vn-overlay-explain-offset-top";
let explainDrag = null;
let explaining = false;
let explainOffset = { x: 0, y: 0 };

try {
  explainOffset = { ...explainOffset, ...JSON.parse(localStorage.getItem(EXPLAIN_PLACE) ?? "{}") };
} catch {
  // Nothing stored, or stored by an older shape. Start where the CSS puts it.
}
applyExplainPlace();

function applyExplainPlace() {
  explainBoxEl.style.setProperty("--ex", `${explainOffset.x}px`);
  explainBoxEl.style.setProperty("--ey", `${explainOffset.y}px`);
  report();
}

/** Clamped like the strip's own drag: pushed off the surface it is gone, and
 *  the surface is the whole screen. */
function moveExplainTo(x, y) {
  const rect = explainBoxEl.getBoundingClientRect();
  const left = rect.left - explainOffset.x;
  const top = rect.top - explainOffset.y;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  explainOffset = {
    x: clamp(x, -left, Math.max(0, window.innerWidth - rect.width) - left),
    y: clamp(y, -top, Math.max(0, window.innerHeight - rect.height) - top),
  };
  applyExplainPlace();
}

explainBtnEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  explainDrag = {
    id: e.pointerId,
    x: e.clientX - explainOffset.x,
    y: e.clientY - explainOffset.y,
    moved: false,
  };
  explainBtnEl.setPointerCapture(e.pointerId);
});

explainBtnEl.addEventListener("pointermove", (e) => {
  if (!explainDrag || e.pointerId !== explainDrag.id) return;
  // A press wanders a pixel or two before it lifts; only a real move is a drag,
  // or the button would stop answering taps.
  if (Math.abs(e.clientX - explainDrag.x - explainOffset.x) > 3) explainDrag.moved = true;
  if (Math.abs(e.clientY - explainDrag.y - explainOffset.y) > 3) explainDrag.moved = true;
  if (explainDrag.moved) moveExplainTo(e.clientX - explainDrag.x, e.clientY - explainDrag.y);
});

for (const type of ["pointerup", "pointercancel"]) {
  explainBtnEl.addEventListener(type, (e) => {
    if (!explainDrag || e.pointerId !== explainDrag.id) return;
    explainBtnEl.releasePointerCapture(e.pointerId);
    const dragged = explainDrag.moved;
    explainDrag = null;
    localStorage.setItem(EXPLAIN_PLACE, JSON.stringify(explainOffset));
    if (type === "pointerup" && !dragged) explainLine();
  });
}

// The widget is its own surface: a click on it must not reach the document
// handler that closes the popup, and must not reach the VN either.
explainBoxEl.addEventListener("click", (e) => e.stopPropagation());

// Reading it is what it is for, so it stays until dismissed — and a click
// anywhere on it dismisses, rather than a ✕ to aim at over a game.
explainPanelEl.addEventListener("click", () => {
  explainPanelEl.hidden = true;
  report();
});

// Take the line off the screen without stopping the overlay: it is over the
// game's own text, and a scene worth looking at is worth looking at whole. The
// stream keeps running, so the line back is whatever is current, not the one
// that was showing when it went.
const hideBtnEl = document.getElementById("hide-btn");
hideBtnEl.addEventListener("click", () => {
  boxEl.hidden = !boxEl.hidden;
  warnEl.hidden = boxEl.hidden;
  hideBtnEl.classList.toggle("off", boxEl.hidden);
  hideBtnEl.title = boxEl.hidden ? "Show the line" : "Hide the line";
  if (boxEl.hidden) closePopup();
  report();
});

// Phone size and back, without restarting the shell: `--mobile` is three query
// parameters, so the toggle flips them and reloads. A reload rather than
// setting the properties live because the layout is read at startup in more
// places than the CSS — the stored drag offset is per layout, and the popup's
// touch buttons are built from `mobile` — and the stream replays the newest
// line the moment it reconnects, so nothing is lost.
const MOBILE_SCALE = 1.75;
const mobileBtnEl = document.getElementById("mobile-btn");
mobileBtnEl.classList.toggle("off", !mobile);
mobileBtnEl.title = mobile ? "Switch to overlay size" : "Switch to phone size";
mobileBtnEl.addEventListener("click", () => {
  const next = new URLSearchParams(location.search);
  // Scaled off what is there rather than set to a constant: `VN_OVERLAY_HEIGHT`
  // and a custom scale both arrive this way, and switching layout must not
  // throw them away.
  const factor = mobile ? 1 / MOBILE_SCALE : MOBILE_SCALE;
  next.set("scale", `${Number(scale) * factor}`);
  next.set("h", `${Math.round(Number(params.get("h") ?? "300") * factor)}`);
  if (mobile) next.delete("mobile");
  else next.set("mobile", "1");
  location.replace(`${location.pathname}?${next}`);
});

// The same switch as `#read`'s: `settings.capture_paused`, which the logger
// polls and answers by closing its Textractor socket. Here because the reason
// to reach for it — a scene not being read, someone else at the keyboard — is
// something that happens while looking at the game, not at the dashboard.
//
// The button reflects the flag rather than a local toggle, so the two pages
// agree: the POST's answer sets it, and every status event corrects it.
const pauseBtnEl = document.getElementById("pause-btn");

function showPaused(paused) {
  pauseBtnEl.classList.toggle("paused", paused);
  pauseBtnEl.textContent = paused ? "▶" : "⏸";
  pauseBtnEl.title = paused ? "Resume capture" : "Pause capture";
}

pauseBtnEl.addEventListener("click", async () => {
  pauseBtnEl.disabled = true;
  try {
    const resp = await fetch("/api/capture/pause", { method: "POST" });
    if (resp.ok) showPaused((await resp.json()).paused);
  } catch {
    // Offline; the next status event says what the flag actually is.
  }
  pauseBtnEl.disabled = false;
});

// How the line is set: size, leading, letter spacing, and how solid the box
// behind it is. Stored in the browser rather than in settings, because they are
// about this screen — a phone reading the same overlay wants its own — and
// applied as CSS variables, so the aligned placement over the game's own text
// keeps working off them.
const TYPE = "vn-overlay-type";
// `?bg=` is the backdrop's starting point, not a competing setting: it is what
// the shell was launched with, and the slider takes over from there.
const TYPE_DEFAULTS = {
  scale: 1,
  leading: 1.68,
  tracking: 0.015,
  backdrop: Number(params.get("bg") ?? 0.82),
};
const TYPE_VARS = {
  scale: (v) => ["--line-scale", `${v}`],
  leading: (v) => ["--line-leading", `${v}`],
  tracking: (v) => ["--line-tracking", `${v}em`],
  backdrop: (v) => ["--backdrop", `rgba(0, 0, 0, ${v})`],
};
let type = { ...TYPE_DEFAULTS };
try {
  type = { ...type, ...JSON.parse(localStorage.getItem(TYPE) ?? "{}") };
} catch {
  // Nothing stored, or stored by an older shape. Draw the line as measured.
}

const settingsBtnEl = document.getElementById("settings-btn");
const settingsPanelEl = document.getElementById("settings-panel");
const typeInputs = {
  scale: [document.getElementById("set-size"), document.getElementById("out-size")],
  leading: [document.getElementById("set-leading"), document.getElementById("out-leading")],
  tracking: [document.getElementById("set-tracking"), document.getElementById("out-tracking")],
  backdrop: [document.getElementById("set-backdrop"), document.getElementById("out-backdrop")],
};

function applyType() {
  for (const [key, value] of Object.entries(type)) {
    const asVar = TYPE_VARS[key];
    if (!asVar) continue;
    root.setProperty(...asVar(value));
    const [input, out] = typeInputs[key];
    input.value = String(value);
    out.textContent = key === "tracking" ? `${value.toFixed(3)}em` : value.toFixed(2);
  }
  localStorage.setItem(TYPE, JSON.stringify(type));
  report();
}

for (const [key, [input]] of Object.entries(typeInputs)) {
  input.addEventListener("input", () => {
    type = { ...type, [key]: Number(input.value) };
    applyType();
  });
}

document.getElementById("settings-reset").addEventListener("click", () => {
  type = { ...TYPE_DEFAULTS };
  applyType();
});

settingsBtnEl.addEventListener("click", () => {
  settingsPanelEl.hidden = !settingsPanelEl.hidden;
  settingsBtnEl.classList.toggle("off", settingsPanelEl.hidden);
  report();
});

applyType();

// The last text selected inside the line, remembered rather than read at the
// press: reaching for the button collapses the selection in the act of
// clicking, so by the time the handler runs there is nothing left to read.
// Cleared with the line it was made in.
let selectedInLine = "";
document.addEventListener("selectionchange", () => {
  const sel = window.getSelection?.();
  const text = (sel?.toString() ?? "").trim();
  if (text && sel.anchorNode && lineEl.contains(sel.anchorNode)) selectedInLine = text;
});

/** Ask the model for a short read on the newest line.
 *
 * Two ways to say which word it should be about, and a selection wins: dragging
 * across part of the line is the only way to ask about something the tokenizer
 * did not make a word. Failing that it is whatever the popup is open on, since
 * on this surface a word is reached by clicking it. */
async function explainLine() {
  if (explaining || !recent.length) return;
  const focus = selectedInLine || (popup.anchor()?.dataset.term ?? "");
  explaining = true;
  explainBtnEl.disabled = true;
  showExplain("…", false);
  try {
    await streamExplain({
      context: recent,
      focus,
      onText: (text) => showExplain(text, false),
    });
  } catch (err) {
    showExplain(err.message, true);
  } finally {
    explaining = false;
    explainBtnEl.disabled = false;
  }
}

/** The model's Markdown as DOM. Built node by node rather than parsed into a
 *  string of HTML: this is model output, and there is no innerHTML on its
 *  path. */
function showExplain(text, isError) {
  const frag = document.createDocumentFragment();
  for (const block of parseMarkdown(text)) {
    if (block.type === "ul") {
      const ul = document.createElement("ul");
      for (const item of block.items) ul.append(inlineMd(item, document.createElement("li")));
      frag.append(ul);
    } else {
      frag.append(inlineMd(block.spans, document.createElement("p")));
    }
  }
  explainPanelEl.replaceChildren(frag);
  explainPanelEl.classList.toggle("err", isError);
  explainPanelEl.hidden = false;
  report();
}

function inlineMd(spans, into) {
  for (const s of spans) {
    if (!s.style) {
      into.append(s.text);
      continue;
    }
    const el = document.createElement(s.style === "bold" ? "strong" : "em");
    el.textContent = s.text;
    into.append(el);
  }
  return into;
}

/** Close, and forget the lookup the popup recorded. */
function closePopup() {
  const word = popup.anchor();
  if (word) word.classList.remove("open");
  popup.close();
  openLookup = null;
}

/** Marking a word known means the popup was opened to reach the button, not to
 * read the definition, so the row it recorded goes.
 *
 * Only known: not knowing a word whose definition is on screen is exactly what
 * a lookup is. Retracted by id rather than re-derived, and nulled after, so
 * this can only ever undo the one row this popup made. */
function onJudged(target, status) {
  if (status !== "known" || openLookup === null) return;
  const lookup_id = openLookup;
  openLookup = null;
  fetch("/api/reader/lookup/retract", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ lookup_id, term: target.term }),
  }).catch(() => {});
}

/** Directly above the line box, centred on the word.
 *
 * Both edges matter for more than looks. The bottom is the *line box's* top,
 * not the word's, so the popup never covers the sentence it came from — and
 * because the two boxes then touch, there is no strip of screen between the
 * word and its definition where a click would reach the VN and advance the
 * line out from under the popup. The left is clamped so a word at either end
 * cannot push the popup off screen.
 */
function place(word) {
  const rect = word.getBoundingClientRect();
  const box = lineEl.getBoundingClientRect();
  const width = popupEl.offsetWidth;
  const left = rect.left + rect.width / 2 - width / 2;
  popupEl.style.left = `${Math.max(12, Math.min(left, window.innerWidth - width - 12))}px`;
  popupEl.style.bottom = `${window.innerHeight - box.top}px`;
  report();
}

/** Known / unknown, written to the same ledger the reading view writes to.
 *
 * The repainted word is the whole report, as in `#read`: no toast, and a
 * failed write is the tint coming back. An expansion has no span of its own to
 * repaint — it spans several — so the button's own state is the report there.
 */
async function judge(word, status, target = null) {
  const on = target ?? { key: word.dataset.term, reading: word.dataset.reading };
  const body = {
    judgements: [{ headword: on.key, reading: on.reading, status }],
  };
  const res = await fetch("/api/vocab/judge", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) return false;
  if (target && target.key !== word.dataset.term) return true;
  word.dataset.status = status;
  word.classList.remove("new", "seen", "unknown");
  if (status !== "known") word.classList.add(status);
  return true;
}

/** The one sentence of `text` that the word at offset `at` is in.
 *
 * A hooked line often holds several sentences, and the card wants the one the
 * word was read in — the same thing Yomitan puts on a card from `#read`. It is
 * also what `vn-trim.py` cuts the voiceline down to: the trim aligns the note's
 * sentence against a transcript of the clip, so a narrower sentence here is a
 * narrower clip, with nothing to change on that side.
 *
 * A newline is not a boundary, unlike `jp_core::text::split_sentences`. The
 * game's own soft breaks arrive as newlines in the line, so treating them as
 * ends would cut a wrapped sentence in half. */
function sentenceAround(text, at) {
  let start = 0;
  for (const end of text.matchAll(/[。！？!?…‥]+[」』）)"”]*/g)) {
    const stop = end.index + end[0].length;
    if (at < stop) return text.slice(start, stop).trim();
    start = stop;
  }
  return text.slice(start).trim();
}

/** A card, built and added the way Yomitan's own add is. Answers with the new
 * note's id, so a popup open on this word can raise its badge now rather than
 * the next time it is opened.
 *
 * Silent otherwise: the chime is the only report a mine gets, here as
 * everywhere, and it plays once the capture and the CompactDef write have both
 * come back. */
async function mine(word, target = null) {
  const on = target ?? {
    key: word.dataset.term,
    reading: word.dataset.reading,
    surface: word.dataset.surface,
    start: Number(word.dataset.start),
  };
  const at = Number.isInteger(on.start) ? on.start : Number(word.dataset.start);
  const res = await fetch("/api/reader/mine", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      term: on.key,
      reading: on.reading,
      surface: on.surface,
      sentence: line ? sentenceAround(line.text, at) : "",
    }),
  });
  // The add answers with the new note's id, so a popup open on this word gets
  // its badge now rather than the next time it is opened.
  const { note_id } = await res.json().catch(() => ({}));
  return note_id;
}

/** Tell the shell every rectangle on this page that should take a click.
 *
 * It hands them to `wl_surface.set_input_region`, so the overlay is clickable
 * exactly where it has drawn something and the VN gets everything else —
 * clicking on to the next line never touches the overlay at all. CSS pixels and
 * window pixels are the same thing here: the view fills the surface.
 *
 * Pushed the moment the layout changes, not polled. Polling meant the region
 * lagged whatever was on screen by a tick or two, and that gap is exactly a
 * click landing on a popup the compositor did not know was there yet — it went
 * to the VN, advanced the line, and closed the popup being aimed at.
 */
function report() {
  if (!shell) return;
  // The whole visible box, not a rectangle per word: anything drawn over the
  // game should swallow the click that lands on it, or a miss between two
  // words advances the VN from under an open popup. It is also far steadier —
  // one rect that changes when the line does, rather than a dozen that shift
  // by a pixel as the text reflows.
  const rects = [explainBoxEl.getBoundingClientRect()];
  if (!boxEl.hidden) rects.push(lineEl.getBoundingClientRect());
  if (!popupEl.hidden) rects.push(popupEl.getBoundingClientRect());
  // Flat `x, y, w, h, ...` rather than nested: an array of arrays reaches Qt
  // as opaque QJSValues, while an array of plain numbers converts cleanly.
  //
  // A few pixels of slack, so a click on the very edge of the backdrop is
  // still caught and the region survives a subpixel reflow.
  shell.setHits(rects.flatMap((r) => [r.left - 4, r.top - 4, r.width + 8, r.height + 8]));
}

// Anything that moves either box without going through `report` itself: the web
// font landing, a long line wrapping, a definition arriving and growing the
// popup upwards.
const watch = new ResizeObserver(report);
watch.observe(lineEl);
watch.observe(popupEl);
watch.observe(explainBoxEl);
window.addEventListener("resize", report);

/* The game moved, resized, appeared or went away. Everything the line is placed
 * with becomes a fraction of this rectangle — see `--text-*` in overlay.html —
 * so following the game costs nothing beyond writing it down.
 *
 * A zero rectangle is "not found", not "at the origin": the game may be
 * Wayland-native, have no window name set on the work, or simply not be running
 * yet. Then the line goes back to sitting against the screen, which is where it
 * sat before any of this. */
function onGeometry(x, y, w, h) {
  game = w > 0 && h > 0 && !mobile ? { x, y, w, h } : null;
  if (game) {
    root.setProperty("--game-x", `${x}px`);
    root.setProperty("--game-y", `${y}px`);
    root.setProperty("--game-w", `${w}px`);
    root.setProperty("--game-h", `${h}px`);
  }
  document.documentElement.toggleAttribute("data-aligned", !!game);
  applyGhost();
  apply();
  // The popup hangs off the line box, so it has to be re-placed under it.
  if (popup.anchor()) place(popup.anchor());
}

// Ghost mode: the line laid over the game's own text and then drawn invisibly,
// so the game does the typesetting and the overlay only marks the words and
// takes the clicks. Only ever on while the game's geometry is known — floating
// marks over nothing is worse than no marks — and never on a phone, where the
// line is being read off the screen rather than fitted over anything.
const GHOST = "vn-overlay-ghost";
const ghostBtnEl = document.getElementById("ghost-btn");
let ghost = localStorage.getItem(GHOST) === "1";

function applyGhost() {
  const on = ghost && !!game;
  document.documentElement.toggleAttribute("data-ghost", on);
  ghostBtnEl.classList.toggle("off", !on);
  // Left pressable while the game is missing and the mode is on: the button is
  // then the only way to turn it back off, and a game that has quit or has not
  // started yet is the ordinary case rather than a fault.
  ghostBtnEl.disabled = !game && !ghost;
  ghostBtnEl.title = game
    ? "Read the game's own text, marked"
    : "Needs the game window — no window name on this work, or it is not running";
  report();
}

function toggleGhost() {
  ghost = !ghost;
  localStorage.setItem(GHOST, ghost ? "1" : "0");
  applyGhost();
}

ghostBtnEl.addEventListener("click", toggleGhost);
applyGhost();

// Only under the overlay shell — in an ordinary browser there is no channel and
// the page is simply a page. qwebchannel.js is injected by the shell, so
// nothing is served for it here.
if (window.qt?.webChannelTransport) {
  new QWebChannel(window.qt.webChannelTransport, (channel) => {
    shell = channel.objects.shell;
    shell.geometry.connect(onGeometry);
    shell.ghostToggled.connect(toggleGhost);
    // The status event that carried it has usually already been and gone.
    if (windowName) shell.setWindowName(windowName);
    report();
  });
}
