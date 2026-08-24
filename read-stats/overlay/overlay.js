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
// ♪ in the popup head plays the word, from the Local Audio Server add-on
// running beside Anki — the same recording Yomitan would play onto a card.
// read-stats proxies it, because that server binds loopback and sends no CORS
// headers, so neither this page nor a phone reading the overlay can ask it.
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
import { THEMES, storedTheme, setTheme } from "/static/lib/theme.js";

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

// The one line that says something is wrong. Status pushes rewrite it every two
// seconds, so anything that has to be *read* — a mine Anki refused — holds it
// for a while against them.
let warnHeldUntil = 0;
function warn(text, holdMs = 0) {
  if (holdMs) warnHeldUntil = Date.now() + holdMs;
  else if (Date.now() < warnHeldUntil) return;
  warnEl.textContent = text;
}
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
    audio: (term, reading) =>
      `/api/reader/audio?${new URLSearchParams({ term, reading })}`,
    audioClip: (path) => `/api/reader/audio/clip?${new URLSearchParams({ path })}`,
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
// The ranks at or under which an unknown word is called common, one per
// frequency list and tested independently. Fetched once; the same settings the
// reading view underlines by, so both agree.
let commonRanks = { freq: 0, bccwj: 0 };
// Paint the ledger's verdict on each word. Off leaves the spans in place —
// they are the click targets — carrying no status class.
let paintStatus = true;
// The gap that ends a sitting, the same one `stats::derive_sessions` and
// `#read` split on — so a divider here marks the same sitting the dashboard
// counts. Ten minutes until the server says otherwise.
let sessionGapSecs = 600;
// Which hooker the capture daemon listens to, and where. Written here and
// polled there — the daemon switches source without being restarted, the same
// way pausing works.
let lineSource = "ws";
let wsUrl = "";
fetch("/api/settings")
  .then((r) => r.json())
  .then((s) => {
    commonRanks = {
      freq: s.reader_common_max_freq_rank || 0,
      bccwj: s.reader_common_max_bccwj_rank || 0,
    };
    paintStatus = s.highlight_status !== false;
    sessionGapSecs = s.session_gap_secs || sessionGapSecs;
    lineSource = s.line_source || "ws";
    wsUrl = s.line_source_ws_url || "";
    showServerSettings();
  })
  .catch(() => {});

// What this installation can do. A control that cannot work is not drawn, so a
// missing part is a smaller overlay rather than a button that fails. Assume
// nothing until the answer arrives: a button appearing a moment late beats one
// that was there and did nothing.
let caps = {};
const can = (name) => caps[name]?.ok === true;
fetch("/api/reader/state")
  .then((r) => r.json())
  .then((s) => {
    caps = s.capabilities ?? {};
    applyCapabilities();
  })
  .catch(() => {});

function applyCapabilities() {
  // No key, no ℹ. The button, not the box: the box is the whole control bar,
  // and pause and the type settings work without a key.
  explainBtnEl.hidden = !can("explain");
  // Nowhere to add a card, no ＋ in the popup.
  popup.setMining(can("anki"));
  // An empty ledger has no verdict to paint, whatever the setting says.
  if (!can("vocabulary_ledger")) paintStatus = false;
}

// A rank at or under a threshold, with 0 meaning the threshold is off.
const underRank = (rank, max) => max > 0 && rank && rank <= max;

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
  const { capture, paused, vn_window } = JSON.parse(e.data);
  // A missing window name is worth saying out loud: the screenshot on a card
  // then grabs the whole screen with the overlay on it, and nothing else here
  // reports that. A capture fault outranks it — no line at all is the bigger
  // problem.
  warn(
    capture !== "live"
      ? capture
      : !can("lines_source")
        ? "no line source — run Textractor with its WebSocket plugin"
        : vn_window
          ? ""
          : "no window name on this work",
  );
  // The flag, not `capture`: the logger takes a poll to close its socket, so
  // `capture` still reads `live` right after a pause and would flip the button
  // back under the click that set it.
  showPaused(paused);
  // Kept even before the channel is up: the first status usually beats it, and
  // the shell is told on connect. The name is per work, so it changes under a
  // running overlay whenever the current work does. Only on a change: the shell
  // answers every push with the game's rectangle, and status arrives every two
  // seconds.
  const name = vn_window ?? "";
  if (name !== windowName) {
    windowName = name;
    shell?.setWindowName(windowName);
  }
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
/** Is this word's status one of the ones being painted? `known` never is — on a
 *  line where most words are known, the absence is what makes the rest
 *  readable. */
function painted(status) {
  if (!paintStatus) return false;
  return { new: type.markNew, seen: type.markSeen, unknown: type.markUnknown }[status] === true;
}

/** Draw the line already on screen again, under settings that decide what is
 *  drawn on it. Not `draw`: that one is the arrival of a line, and would send
 *  it to the explain context a second time. */
function redraw() {
  if (line) draw(line, true);
}

/** One line, built the way the live line is built: the same word spans, the
 * same status marks, the same common-word underline, the same ruby.
 *
 * Shared with the scrollback, which is the reason it is a function rather than
 * the body of `draw`. A line the reader scrolled back to is the same line it
 * was a minute ago, and a second implementation would drift from this one
 * exactly where it matters — what a word is worth knowing about. */
function renderLine(row) {
  const text = row.text;
  const ruby = row.ruby ?? [];
  const frag = document.createDocumentFragment();
  // Offsets are UTF-16 code units, which is exactly what a JS string indexes
  // in, so they slice directly.
  const parts = pieces(text, [...(row.tokens ?? [])].sort((a, b) => a.start - b.start));
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
      // No frequency dictionary means no answer to "is this word common", so
      // nothing is underlined rather than everything reading as rare.
      const common =
        can("dict_frequency") &&
        (underRank(span.freq_rank, commonRanks.freq) ||
          underRank(span.bccwj_rank, commonRanks.bccwj)) &&
        span.status !== "known";
      const mark = painted(span.status) ? span.status : "";
      word.className = ["w", mark, common ? "common" : ""].filter(Boolean).join(" ");
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

  return frag;
}

function draw(incoming, again = false) {
  closePopup();
  line = incoming;
  if (!again) {
    recent.push(incoming.text);
    if (recent.length > EXPLAIN_CONTEXT_LINES) recent.shift();
  }
  selectedInLine = "";
  lineEl.replaceChildren(renderLine(line));
  // The game indents a quoted line's later rows under its first character, not
  // under the 「 — see #line.quoted. Reading it off the text rather than always
  // hanging the indent: a narration line starts at the margin and every row of
  // it does.
  lineEl.classList.toggle("quoted", QUOTE_OPEN.test(line.text));
  if (scrollbackOpen()) appendScrollback(line);
  report();
}

function onWordClick(e) {
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
}

// Anywhere else on the surface dismisses. Not the popup itself, or scrolling a
// long Jitendex entry would close what is being read.
//
// The popup stops its own clicks rather than the handler below testing where
// they came from. It used to test, and `closest("#popup")` answers about where
// the target is *now*: picking another match re-renders the popup from inside
// the click, which detaches the chip mid-dispatch, and the detached chip then
// read as a click outside — so every pick closed the popup it had just opened.
document.addEventListener("click", () => closePopup());

// Escape closes the popup, then the scrollback. The layer surface only takes
// the keyboard once it has been clicked, which by then it has been — opening
// the scrollback is a click on it.
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (popup.isOpen()) return closePopup();
  closeScrollback();
});

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

function onWordMousedown(e) {
  if (SIDE_ACTIONS[e.button]) e.preventDefault();
}

function onWordAuxclick(e) {
  const action = SIDE_ACTIONS[e.button];
  const word = e.target.closest(".w");
  if (!action || !word) return;
  e.preventDefault();
  action(word);
}

// The wheel over the word the popup is open on pages its dictionaries — the
// hand is already there, having just clicked it. Only over that word: anywhere
// else the wheel still scrolls a line too long for the strip.
function onWordWheel(e) {
  if (!popup.isOpen() || e.target.closest(".w") !== popup.anchor()) return;
  e.preventDefault();
  popup.step(Math.sign(e.deltaY));
}


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

// What `apply` last put on the box. The box's measured position includes it,
// so subtracting it back out is what gives the position with no drag at all —
// and it is not always `offsetPx`, because an offset that would put the box
// outside the surface is clamped rather than obeyed.
let applied = { x: 0, y: 0 };

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

/** A drag in pixels, held where the box can still be seen and grabbed.
 *
 * Clamped on the way *out* rather than only when dragged: the surface is not a
 * fixed size. It is the game's window, and the game is resized, goes
 * fullscreen, or is replaced by one a different shape — each of which can leave
 * a stored offset pointing past an edge that has moved. An offset that does
 * that is a strip drawn outside the surface, which is not a strip drawn partly
 * off the edge but one that is not drawn at all.
 *
 * The stored value is left alone. It is a calibration against the game's own
 * text, so it is still the right one when the surface it was measured on comes
 * back — clamping is how it is *drawn* meanwhile, not a correction to it. */
function clampPx(at) {
  const rect = lineEl.getBoundingClientRect();
  // Before the first layout there is nothing to hold inside anything.
  if (!rect.width && !rect.height) return at;
  const left = rect.left - applied.x;
  const top = rect.top - applied.y;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  return {
    x: clamp(at.x, -left, Math.max(0, window.innerWidth - rect.width) - left),
    y: clamp(at.y, -top, Math.max(0, window.innerHeight - rect.height) - top),
  };
}

function moveTo(x, y) {
  const px = clampPx({ x, y });
  if (game) alignedOffset = { x: px.x / game.w, y: px.y / game.h };
  else offset = px;
  apply();
}

function apply() {
  applied = clampPx(offsetPx());
  boxEl.style.setProperty("--dx", `${applied.x}px`);
  boxEl.style.setProperty("--dy", `${applied.y}px`);
  report();
}

// The surface is resized under the page: it is the game's window, and the game
// is resized, goes fullscreen, or is replaced by one a different shape. The
// shell says so through `geometry`, but it says so *before* the compositor has
// configured the surface — so the viewport is still the old size when that
// arrives, and clamping against it there clamps against nothing. This is the
// event that means the new size is real.
window.addEventListener("resize", () => {
  apply();
  if (scrollbackOpen()) sizeScrollback();
  if (popup.anchor()) place(popup.anchor());
});

lineEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || e.target.closest(".w")) return;
  // Grab from wherever the box currently sits, which aligned to the game is
  // the fraction scaled up — not the free-floating offset, which is then zero.
  const at = offsetPx();
  drag = { id: e.pointerId, x: e.clientX - at.x, y: e.clientY - at.y, moved: false };
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
const handleEl = document.getElementById("bar-handle");
const buttonsEl = document.getElementById("buttons");
const explainPanelEl = document.getElementById("explain-panel");
// Versioned: the widget used to hang off the bottom edge, and an offset stored
// against that anchor puts it somewhere else entirely against this one.
const EXPLAIN_PLACE = "vn-overlay-explain-offset-top";
let barDrag = null;
let barDragged = false;
let explaining = false;
let explainOffset = { x: 0, y: 0 };
// As `applied` is to the strip's offset: what was last put on the widget, which
// is not always what is stored — see `clampExplainPx`.
let explainApplied = { x: 0, y: 0 };

try {
  explainOffset = { ...explainOffset, ...JSON.parse(localStorage.getItem(EXPLAIN_PLACE) ?? "{}") };
} catch {
  // Nothing stored, or stored by an older shape. Start where the CSS puts it.
}
applyExplainPlace();

/** Held on the surface, like the strip's own offset and for the same reason:
 *  pushed off it the widget is not drawn at all, and what is not drawn cannot
 *  be dragged back. Clamped where it is *used*, because what it is measured
 *  against moves — the widget hangs off the game's corner now, and the game
 *  is moved, resized and replaced. */
function clampExplainPx(at) {
  const rect = explainBoxEl.getBoundingClientRect();
  if (!rect.width && !rect.height) return at;
  const left = rect.left - explainApplied.x;
  const top = rect.top - explainApplied.y;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  return {
    x: clamp(at.x, -left, Math.max(0, window.innerWidth - rect.width) - left),
    y: clamp(at.y, -top, Math.max(0, window.innerHeight - rect.height) - top),
  };
}

function applyExplainPlace() {
  explainApplied = clampExplainPx(explainOffset);
  explainBoxEl.style.setProperty("--ex", `${explainApplied.x}px`);
  explainBoxEl.style.setProperty("--ey", `${explainApplied.y}px`);
  report();
}

function moveExplainTo(x, y) {
  explainOffset = clampExplainPx({ x, y });
  applyExplainPlace();
}

handleEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  barDrag = {
    id: e.pointerId,
    x: e.clientX - explainOffset.x,
    y: e.clientY - explainOffset.y,
    moved: false,
  };
  handleEl.setPointerCapture(e.pointerId);
});

handleEl.addEventListener("pointermove", (e) => {
  if (!barDrag || e.pointerId !== barDrag.id) return;
  // A press wanders a pixel or two before it lifts; only a real move is a drag,
  // or the handle would stop answering taps.
  if (Math.abs(e.clientX - barDrag.x - explainOffset.x) > 3) barDrag.moved = true;
  if (Math.abs(e.clientY - barDrag.y - explainOffset.y) > 3) barDrag.moved = true;
  if (barDrag.moved) moveExplainTo(e.clientX - barDrag.x, e.clientY - barDrag.y);
});

for (const type of ["pointerup", "pointercancel"]) {
  handleEl.addEventListener(type, (e) => {
    if (!barDrag || e.pointerId !== barDrag.id) return;
    handleEl.releasePointerCapture(e.pointerId);
    barDragged = barDrag.moved;
    barDrag = null;
    localStorage.setItem(EXPLAIN_PLACE, JSON.stringify(explainOffset));
  });
}

// The toggle is the click, not the pointerup that ends the drag: a press with
// the pointer captured does not always lift on the button it went down on, and
// the bar then stopped answering. A drag that actually moved is not a click.
handleEl.addEventListener("click", () => {
  if (barDragged) {
    barDragged = false;
    return;
  }
  showBar(buttonsEl.hidden);
});

// The bar is shut while the game is being read: every button on it is something
// in the way of the art, and the handle is the one thing that has to stay — it
// is also what the widget is dragged by. Only the handle opens and shuts it.
function showBar(open) {
  buttonsEl.hidden = !open;
  if (!open) closeBarPanels(null);
  report();
}

explainBtnEl.addEventListener("click", explainLine);

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
// The button row's tooltip is drawn by the page — see `[data-tip]` in
// overlay.html — so `title` must stay unset or the native one draws too.
const tip = (el, text) => el.setAttribute("data-tip", text);

/* Earlier lines, over the whole surface.
 *
 * A page of history is fetched only when the top is reached, so opening it
 * costs one request and scrolling back a thousand lines costs one per page.
 * The rows are built by `renderLine`, so every word in them clicks, judges and
 * mines exactly as the live line's do — a lookup from here is a lookup, which
 * is the point: a word met three lines ago is the commonest thing to want to
 * look up, and reaching it used to mean not looking it up at all.
 */
const scrollbackEl = document.getElementById("scrollback");
const scrollbackLinesEl = document.getElementById("scrollback-lines");
const scrollbackCountEl = document.getElementById("scrollback-count");
const scrollbackBtnEl = document.getElementById("scrollback-btn");

// The oldest line held, and whether the server has said there is nothing older.
let oldestId = null;
let exhausted = false;
let paging = false;

const scrollbackOpen = () => !scrollbackEl.hidden;

function scrollbackRow(row) {
  const el = document.createElement("div");
  el.className = "sb";
  el.dataset.id = String(row.id ?? "");
  el.dataset.ts = String(row.ts ?? 0);
  el.append(renderLine(row));
  return el;
}

/** "14:32". en-GB like the sittings table, since the default locale adds an
 *  AM/PM and this sits in a narrow column over a game. */
function clock(ts) {
  return new Date(ts * 1000).toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Rule the dividers off the rows: a gap over `sessionGapSecs` starts a new
 *  sitting, which is what the server and `#read` both split on, so a divider
 *  here marks the same sitting the dashboard counts.
 *
 *  Redone from scratch on every change rather than patched. A line arriving can
 *  close the sitting before it — turning its "started 22:26" into a finished
 *  range — and a page loaded above can reveal that what looked like the top of
 *  a sitting is the middle of one. Walking a few hundred rows costs nothing
 *  next to getting either of those wrong.
 *
 *  The oldest sitting held gets no header until there is nothing older, since
 *  until then its first line is only the first line *loaded*. */
function markSessions() {
  for (const old of scrollbackLinesEl.querySelectorAll(".sb-session")) old.remove();
  const rows = [...scrollbackLinesEl.querySelectorAll(".sb")];
  const groups = [];
  for (const row of rows) {
    const ts = Number(row.dataset.ts) || 0;
    const group = groups[groups.length - 1];
    if (!group || ts - group.end > sessionGapSecs) groups.push({ first: row, start: ts, end: ts });
    else group.end = ts;
  }
  groups.forEach((group, i) => {
    if (i === 0 && !exhausted) return;
    const header = document.createElement("div");
    header.className = "sb-session";
    // The last one is still being read: there is no closed flag, only the
    // absence of a next line so far.
    header.textContent =
      i === groups.length - 1
        ? `started ${clock(group.start)}`
        : `${clock(group.start)}–${clock(group.end)}`;
    group.first.before(header);
  });
}

function appendScrollback(row) {
  // Only when it is already at the bottom: a reader who has scrolled up to read
  // something is not asking to be dragged back down every time the game
  // advances.
  const atBottom =
    scrollbackLinesEl.scrollHeight - scrollbackLinesEl.scrollTop - scrollbackLinesEl.clientHeight < 40;
  for (const el of scrollbackLinesEl.querySelectorAll(".sb.current")) el.classList.remove("current");
  const el = scrollbackRow(row);
  el.classList.add("current");
  scrollbackLinesEl.append(el);
  markSessions();
  if (atBottom) toLatest();
  countScrollback();
}

/** The newest line, which is the bottom. Not `scrollIntoView`: that scrolls
 *  every scrollable ancestor, and the panel is inside the surface now. */
function toLatest() {
  scrollbackLinesEl.scrollTop = scrollbackLinesEl.scrollHeight;
}

function countScrollback() {
  // The rows, not the children: the session dividers are children too.
  const n = scrollbackLinesEl.querySelectorAll(".sb").length;
  const more = exhausted ? "" : ", scroll up for more";
  scrollbackCountEl.textContent = n ? `${n} lines${more}` : "Nothing read yet";
}

/** One page older than what is held. Anchored on the oldest id rather than an
 *  offset: lines keep arriving while this is open, and an offset would slide. */
async function pageBack() {
  if (paging || exhausted || oldestId === null) return;
  paging = true;
  // Held so the view can be put back where it was: prepending changes
  // scrollHeight, and without this the reader is thrown to a random place.
  const before = scrollbackLinesEl.scrollHeight - scrollbackLinesEl.scrollTop;
  try {
    const res = await fetch(`/api/lines/before?before=${oldestId}&limit=100`);
    const { lines: older = [] } = await res.json();
    if (!older.length) {
      exhausted = true;
      return;
    }
    oldestId = older[0].id;
    const frag = document.createDocumentFragment();
    for (const row of older) frag.append(scrollbackRow(row));
    scrollbackLinesEl.prepend(frag);
    markSessions();
    scrollbackLinesEl.scrollTop = scrollbackLinesEl.scrollHeight - before;
  } catch {
    // Offline or the server restarted. Leave what is held and let the next
    // scroll try again.
  } finally {
    paging = false;
    // Again, because exhausting the history is what lets the oldest sitting
    // have a header at all.
    markSessions();
    countScrollback();
  }
}

/** Match the line's column in *characters*, whichever way #box is being sized.
 *
 * A count rather than the measured width: the panel is set smaller than the
 * line, so copying the pixels would draw a column far wider than it needs and
 * still rewrap nothing. Passing the count lets the CSS re-measure it at this
 * panel's own type.
 *
 * Read rather than derived: aligned to the game the column is a character
 * count, and free-floating it is a pair of viewport insets. A hidden line has
 * no width to count, so the CSS fallback stands. */
function sizeScrollback() {
  const style = getComputedStyle(lineEl);
  const width =
    lineEl.getBoundingClientRect().width -
    parseFloat(style.paddingLeft) -
    parseFloat(style.paddingRight);
  const advance = parseFloat(style.fontSize) + (parseFloat(style.letterSpacing) || 0);
  if (width > 100 && advance > 0) {
    root.setProperty("--sb-chars", `${Math.round(width / advance)}`);
  } else {
    root.removeProperty("--sb-chars");
  }
}

function openScrollback() {
  scrollbackEl.hidden = false;
  sizeScrollback();
  scrollbackBtnEl.classList.remove("off");
  if (!scrollbackLinesEl.children.length) {
    // Seeded from the line on screen, which is the only id this page is sure
    // of. Everything older arrives by paging back from it.
    if (line) {
      oldestId = line.id ?? null;
      scrollbackLinesEl.append(scrollbackRow(line));
      scrollbackLinesEl.lastElementChild.classList.add("current");
    }
    pageBack();
  }
  markSessions();
  countScrollback();
  toLatest();
  report();
}

function closeScrollback() {
  if (!scrollbackOpen()) return;
  scrollbackEl.hidden = true;
  scrollbackBtnEl.classList.add("off");
  closePopup();
  report();
}

scrollbackBtnEl.addEventListener("click", () => {
  scrollbackOpen() ? closeScrollback() : openScrollback();
});

/** The three things that hang under the bar are alternatives, not a stack.
 *
 * Each is the answer to a different question — what was said before this, what
 * does this line mean, how should the line look — and none of them is read
 * while another is. Left open together they are three panels of clutter over
 * the game, and the lower ones are unreachable behind the taller ones anyway.
 */
function closeBarPanels(keep) {
  if (keep !== scrollbackEl) closeScrollback();
  if (keep !== explainPanelEl) explainPanelEl.hidden = true;
  if (keep !== settingsPanelEl) {
    settingsPanelEl.hidden = true;
    settingsBtnEl.classList.add("off");
  }
}

// Which button owns which panel, by id — the elements are not all declared
// yet here. A button in neither column — the hide, the pause and the handle —
// owns nothing and closes all three.
const BAR_PANELS = new Map([
  ["scrollback-btn", "scrollback"],
  ["explain-btn", "explain-panel"],
  ["settings-btn", "settings-panel"],
]);

// After the buttons' own handlers, which run at the target and are what open
// the panel this then keeps.
document.getElementById("bar").addEventListener("click", (e) => {
  const button = e.target.closest("button");
  if (!button) return;
  const keep = BAR_PANELS.get(button.id);
  closeBarPanels(keep ? document.getElementById(keep) : null);
  report();
});
document.getElementById("scrollback-latest").addEventListener("click", () => toLatest());

scrollbackLinesEl.addEventListener("scroll", () => {
  if (scrollbackLinesEl.scrollTop < 200) pageBack();
});

// Its own clicks stay inside it: it sits under the bar, and the document-level
// dismiss below would otherwise close the popup a word in here has just opened.
scrollbackEl.addEventListener("click", (e) => e.stopPropagation());

// The wheel inside it scrolls it rather than reaching the game, and
// `overscroll-behavior` keeps a scroll that hits the end from leaving.
scrollbackEl.addEventListener("wheel", (e) => e.stopPropagation(), { passive: false });

// The live line and the scrollback carry the same word spans, so they carry the
// same handlers — not copies. A word is a word wherever it is drawn, and the
// scrollback exists precisely so a line that has gone past can still be looked
// up.
for (const host of [lineEl, scrollbackLinesEl]) {
  host.addEventListener("click", onWordClick);
  host.addEventListener("mousedown", onWordMousedown);
  host.addEventListener("auxclick", onWordAuxclick);
  host.addEventListener("wheel", onWordWheel, { passive: false });
}

const hideBtnEl = document.getElementById("hide-btn");
hideBtnEl.addEventListener("click", () => {
  boxEl.hidden = !boxEl.hidden;
  warnEl.hidden = boxEl.hidden;
  hideBtnEl.classList.toggle("off", boxEl.hidden);
  tip(hideBtnEl, boxEl.hidden ? "Show the line" : "Hide the line");
  if (boxEl.hidden) closePopup();
  report();
});

// Phone size and back, without restarting the shell: `--mobile` is three query
// parameters, so the toggle flips them and reloads. A reload rather than
// setting the properties live because the layout is read at startup in more
// places than the CSS — the stored drag offset is per layout, and the popup's
// touch buttons are built from `mobile` — and the stream replays the newest
// line the moment it reconnects, so nothing is lost.
const mobileBtnEl = document.getElementById("mobile-btn");
mobileBtnEl.classList.toggle("off", !mobile);
tip(mobileBtnEl, mobile ? "Switch to overlay size" : "Switch to phone size");
mobileBtnEl.addEventListener("click", () => {
  const next = new URLSearchParams(location.search);
  // Scaled off what is there rather than set to a constant: `VN_OVERLAY_HEIGHT`
  // and a custom scale both arrive this way, and switching layout must not
  // throw them away.
  const factor = mobile ? 1 / type.mobileScale : type.mobileScale;
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
  // The one state worth seeing with the bar shut.
  handleEl.classList.toggle("paused", paused);
  // A glyph, not a phrase: it sits in the row of square buttons over the game
  // now, and the tooltip is where the sentence goes.
  pauseBtnEl.textContent = paused ? "▶" : "⏸";
  tip(pauseBtnEl, paused ? "Resume capture" : "Pause capture");
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

// The overlay's settings, in three tabs: how the line is set and sized, what
// is marked on it, and where its lines come from.
//
// Most of them are stored in the browser, because they are about this screen —
// a phone reading the same overlay wants its own — and applied as CSS
// variables, so the aligned placement over the game's own text keeps working
// off them. The two that every reading surface has to agree on — whether a
// status is painted at all, and the rank an unknown word counts as common
// under — are read from and written back to read-stats.
const TYPE = "vn-overlay-type";
// `?bg=` and `?h=` are starting points, not competing settings: they are what
// the shell was launched with, and the panel takes over from there.
const TYPE_DEFAULTS = {
  scale: 1,
  leading: 1.68,
  tracking: 0,
  weight: 400,
  backdrop: Number(params.get("bg") ?? 0.82),
  shadow: 2,
  shadowBlur: 3,
  // Empty means the launcher's `?font=`, left where overlay.js put it above.
  font: "",
  // The ink, as HSL — no saturation at full lightness is white, which is what
  // the line was measured in.
  hue: 0,
  sat: 0,
  light: 100,
  chars: 40,
  // What the phone-size toggle scales by. No control: one factor reads well on
  // a phone, and a slider for it was a setting nobody moved twice.
  mobileScale: 1.75,
  tint: 0.85,
  markNew: true,
  markSeen: true,
  markUnknown: true,
};

const TYPE_VARS = {
  scale: (v) => ["--line-scale", `${v}`],
  leading: (v) => ["--line-leading", `${v}`],
  tracking: (v) => ["--line-tracking", `${v}em`],
  weight: (v) => ["--line-weight", `${v}`],
  backdrop: (v) => ["--backdrop", `hsl(0 0% 0% / ${v})`],
  chars: (v) => ["--text-chars", `${v}`],
  tint: (v) => ["--tint", `${v}`],
};

/** How each control reads and writes its setting. `fmt` is the readout beside a
 *  slider; a checkbox has none. */
const CONTROLS = {
  scale: { id: "set-size", out: "out-size", fmt: (v) => v.toFixed(2) },
  leading: { id: "set-leading", out: "out-leading", fmt: (v) => v.toFixed(2) },
  tracking: { id: "set-tracking", out: "out-tracking", fmt: (v) => `${v.toFixed(3)}em` },
  weight: { id: "set-weight", out: "out-weight", fmt: (v) => `${v}` },
  backdrop: { id: "set-backdrop", out: "out-backdrop", fmt: (v) => v.toFixed(2) },
  shadow: { id: "set-shadow", out: "out-shadow", fmt: (v) => (v ? `${v}x` : "off") },
  shadowBlur: { id: "set-shadow-blur", out: "out-shadow-blur", fmt: (v) => `${v}px` },
  hue: { id: "set-hue", out: "out-hue", fmt: (v) => `${v}°` },
  sat: { id: "set-sat", out: "out-sat", fmt: (v) => `${v}%` },
  light: { id: "set-light", out: "out-light", fmt: (v) => `${v}%` },
  // Redrawn as it moves: the width is where the line breaks, and the break is
  // the thing being set.
  chars: { id: "set-chars", out: "out-chars", fmt: (v) => `${v}`, repaints: true },
  tint: { id: "set-tint", out: "out-tint", fmt: (v) => v.toFixed(2) },
  markNew: { id: "set-mark-new", check: true, repaints: true },
  markSeen: { id: "set-mark-seen", check: true, repaints: true },
  markUnknown: { id: "set-mark-unknown", check: true, repaints: true },
};

let type = { ...TYPE_DEFAULTS };
try {
  const stored = JSON.parse(localStorage.getItem(TYPE) ?? "{}");
  // Key by key, and only where the stored value is still the same kind of
  // thing. The shell keeps localStorage across releases, so a setting that
  // changes shape — the shadow was a checkbox and is a strength — arrives as
  // the old kind and would throw the moment its readout is formatted, taking
  // the rest of this file's setup with it: no input region, and an overlay
  // nothing can be clicked on.
  for (const [key, value] of Object.entries(stored)) {
    if (key in TYPE_DEFAULTS && typeof value === typeof TYPE_DEFAULTS[key]) type[key] = value;
  }
} catch {
  // Nothing stored, or not JSON at all. Draw the line as measured.
}

const settingsBtnEl = document.getElementById("settings-btn");
const settingsPanelEl = document.getElementById("settings-panel");
const fontBoxEl = document.getElementById("set-font");
let fontBtnEls = [];

// Every Japanese-capable family fontconfig knows about, which only the server
// can ask. Until it answers — or if it answers with nothing — the list is the
// one entry that always works: whatever the shell was launched with.
addFonts([]);
fetch("/api/reader/fonts")
  .then((r) => r.json())
  .then((f) => addFonts(f.families ?? []))
  .catch(() => {});

function addFonts(families) {
  fontBoxEl.replaceChildren();
  for (const family of ["", ...families]) {
    const row = document.createElement("div");
    row.className = "row";
    row.dataset.family = family;
    if (family) {
      const sample = document.createElement("span");
      sample.className = "sample";
      sample.textContent = "あア亜";
      sample.style.fontFamily = `"${family}", sans-serif`;
      row.append(sample);
    }
    row.append(family || "As launched");
    fontBoxEl.append(row);
    row.addEventListener("click", () => {
      type = { ...type, font: family };
      applyType();
    });
  }
  fontBtnEls = [...fontBoxEl.children];
  for (const row of fontBtnEls) row.classList.toggle("on", row.dataset.family === type.font);
}

function applyType() {
  for (const row of fontBtnEls) row.classList.toggle("on", row.dataset.family === type.font);
  root.setProperty("--line-font", `"${type.font || font || "Noto Sans CJK JP"}", sans-serif`);
  root.setProperty("--line-color", `hsl(${type.hue} ${type.sat}% ${type.light}%)`);
  // Centred on the glyphs rather than dropped below them: this sits over
  // artwork, and what the shadow is for is lifting the character off whatever
  // is behind it, not casting it in a direction.
  //
  // Strength is how many times the same shadow is drawn, not how opaque it is.
  // A single blurred shadow at full opacity is still faint — the blur spreads
  // what it has over its whole radius — so opacity is the wrong knob and stacked
  // copies are what actually darkens it.
  const shade = `0 0 ${type.shadowBlur}px hsl(0 0% 0%)`;
  // Rounded: a value stored when this was an opacity is a fraction, and
  // `Array` throws on one.
  const layers = Math.max(0, Math.round(type.shadow));
  root.setProperty(
    "--line-shadow",
    layers > 0 ? Array(layers).fill(shade).join(", ") : "none",
  );
  for (const [key, value] of Object.entries(type)) {
    const asVar = TYPE_VARS[key];
    if (asVar) root.setProperty(...asVar(value));
    const control = CONTROLS[key];
    if (!control) continue;
    const input = document.getElementById(control.id);
    if (control.check) input.checked = !!value;
    else {
      input.value = String(value);
      if (control.out) document.getElementById(control.out).textContent = control.fmt(value);
    }
  }
  localStorage.setItem(TYPE, JSON.stringify(type));
  report();
}

for (const [key, control] of Object.entries(CONTROLS)) {
  const input = document.getElementById(control.id);
  input.addEventListener("input", () => {
    type = { ...type, [key]: control.check ? input.checked : Number(input.value) };
    applyType();
    // Which statuses are painted is decided while the line is being built, so
    // the line on screen has to be built again.
    if (control.repaints) redraw();
  });
}

// Light, dark, or whatever the machine says — the dashboard's own control, on
// the dashboard's own key, so the setting means one thing wherever it is
// changed. The stamp is already on <html> from the inline script in the page
// head; this only offers the three states and remembers a new pick. The line
// itself is untouched by it: what colour that is drawn in and how dark its
// backdrop sits are the type settings above, because the line is registered
// against the game's own text rather than against a page.
const themeBoxEl = document.getElementById("set-theme");
const themeBtnEls = [...themeBoxEl.children];

function showTheme(theme) {
  for (const btn of themeBtnEls) btn.classList.toggle("on", btn.value === theme);
}

for (const btn of themeBtnEls) {
  btn.addEventListener("click", () => {
    if (!THEMES.includes(btn.value)) return;
    setTheme(btn.value);
    showTheme(btn.value);
  });
}
showTheme(storedTheme());

// One tab at a time.
const tabBtnEls = [...document.querySelectorAll("#settings-tabs button")];
const tabBodyEls = [...document.querySelectorAll(".settings-body")];

for (const btn of tabBtnEls) {
  btn.addEventListener("click", () => {
    for (const other of tabBtnEls) other.classList.toggle("on", other === btn);
    for (const body of tabBodyEls) body.hidden = body.dataset.tab !== btn.value;
    report();
  });
}

settingsBtnEl.addEventListener("click", () => {
  settingsPanelEl.hidden = !settingsPanelEl.hidden;
  settingsBtnEl.classList.toggle("off", settingsPanelEl.hidden);
  report();
});

applyType();

// The two settings read-stats owns, written back as they are changed: `#read`
// underlines by the same rank and paints by the same flag, and a switch that
// only moved this page would make the two surfaces disagree about the same
// word.
const statusInputEl = document.getElementById("set-status");
const commonInputEl = document.getElementById("set-common");

const sourceBoxEl = document.getElementById("set-line-source");
const sourceRowEls = [...sourceBoxEl.children];
const wsUrlEl = document.getElementById("set-ws-url");
const sourceNoteEl = document.getElementById("source-note");

function showServerSettings() {
  statusInputEl.checked = paintStatus;
  commonInputEl.value = String(commonRanks.freq);
  for (const row of sourceRowEls) row.classList.toggle("on", row.dataset.source === lineSource);
  // Not while it is being typed in: rewriting the box under the cursor moves
  // the caret to the end of whatever has been typed so far.
  if (document.activeElement !== wsUrlEl) wsUrlEl.value = wsUrl;
  wsUrlEl.disabled = lineSource !== "ws";
  sourceNoteEl.textContent =
    lineSource === "clipboard"
      ? "Anything copied is read as a line, including a sentence copied for a lookup. Needs wl-clipboard or xclip."
      : "Textractor's WebSocket plugin, which is the port set in Textractor itself.";
}

function saveSetting(key, value) {
  fetch("/api/settings", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ [key]: value }),
  }).catch(() => {
    // Offline. The panel still shows what was asked for; the next load reads
    // back whatever was actually stored.
  });
}

function setPaintStatus(on) {
  paintStatus = on;
  statusInputEl.checked = on;
  saveSetting("highlight_status", on);
  redraw();
}

function setCommonRank(rank) {
  commonRanks = { ...commonRanks, freq: rank };
  commonInputEl.value = String(rank);
  saveSetting("reader_common_max_freq_rank", rank);
  redraw();
}

function setLineSource(source) {
  lineSource = source;
  showServerSettings();
  saveSetting("line_source", source);
}

for (const row of sourceRowEls) {
  row.addEventListener("click", () => setLineSource(row.dataset.source));
}

// On change, not on input: the address is only valid once it has been typed
// out, and saving each keystroke would point the daemon at ws://l for a moment.
wsUrlEl.addEventListener("change", () => {
  wsUrl = wsUrlEl.value.trim();
  saveSetting("line_source_ws_url", wsUrl);
});

statusInputEl.addEventListener("change", () => setPaintStatus(statusInputEl.checked));
commonInputEl.addEventListener("change", () => setCommonRank(Math.max(0, Number(commonInputEl.value) || 0)));

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
  // Clear of the whole line box for a word in the live line, so the popup never
  // covers the line it came from. Clear of the *word* in the history panel:
  // that box is fourteen rows tall, and clearing all of it would push every
  // popup off the top of the screen.
  const anchor = scrollbackEl.contains(word) ? rect : lineEl.getBoundingClientRect();
  const width = popupEl.offsetWidth;
  const height = popupEl.offsetHeight;
  const left = rect.left + rect.width / 2 - width / 2;
  popupEl.style.left = `${Math.max(12, Math.min(left, window.innerWidth - width - 12))}px`;
  // Above where there is room, below where there is not. The history panel
  // hangs from the top of the screen, so its first rows have nothing above
  // them — and a popup pinned above them would be drawn off-screen.
  if (anchor.top >= height + 16) {
    popupEl.style.top = "auto";
    popupEl.style.bottom = `${window.innerHeight - anchor.top}px`;
  } else {
    popupEl.style.bottom = "auto";
    popupEl.style.top = `${anchor.bottom + 8}px`;
  }
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
  const { note_id, error } = await res.json().catch(() => ({}));
  // Anki refuses in its own words and with a 200, so this is the only place the
  // reason exists. Held on screen: a mine that quietly does nothing is
  // indistinguishable from a click that missed, and the reason is usually
  // something only the reader can fix — the wrong profile is open, the note
  // type was renamed.
  if (!note_id) warn(error ? `Anki: ${error}` : "Anki added no card", 12000);
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
  // Its own rectangle even though it is inside #explain-box: it is as wide as
  // the line's column, which is wider than the box's max-width, and the part
  // sticking out would take no clicks — they would land on the game and
  // advance it under the panel being read.
  if (scrollbackOpen()) rects.push(scrollbackEl.getBoundingClientRect());
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
  // The line's column moves with the game, and the panel is that column.
  if (scrollbackOpen()) queueMicrotask(sizeScrollback);
  if (game) {
    root.setProperty("--game-x", `${x}px`);
    root.setProperty("--game-y", `${y}px`);
    root.setProperty("--game-w", `${w}px`);
    root.setProperty("--game-h", `${h}px`);
  }
  document.documentElement.toggleAttribute("data-aligned", !!game);
  applyGhost();
  apply();
  // The widget's corner is the game's corner now, so it has moved too — and
  // its offset is clamped against where it has moved to.
  applyExplainPlace();
  // The popup hangs off the line box, so it has to be re-placed under it.
  if (popup.anchor()) place(popup.anchor());
}

// Ghost mode: the line laid over the game's own text and then drawn invisibly,
// so the game does the typesetting and the overlay only marks the words and
// takes the clicks. Only ever on while the game's geometry is known — floating
// marks over nothing is worse than no marks — and never on a phone, where the
// line is being read off the screen rather than fitted over anything.
const GHOST = "vn-overlay-ghost";
const ghostInputEl = document.getElementById("set-ghost");
const ghostWhyEl = document.getElementById("ghost-why");
let ghost = localStorage.getItem(GHOST) === "1";

function applyGhost() {
  const on = ghost && !!game;
  document.documentElement.toggleAttribute("data-ghost", on);
  ghostInputEl.checked = ghost;
  // Left settable while the game is missing and the mode is on: the checkbox is
  // then the only way to turn it back off, and a game that has quit or has not
  // started yet is the ordinary case rather than a fault.
  ghostInputEl.disabled = !game && !ghost;
  ghostWhyEl.textContent = game ? "" : "needs the game window";
  report();
}

function setGhost(on) {
  ghost = on;
  localStorage.setItem(GHOST, ghost ? "1" : "0");
  applyGhost();
}

function toggleGhost() {
  setGhost(!ghost);
}

ghostInputEl.addEventListener("change", () => setGhost(ghostInputEl.checked));
applyGhost();


// Only under the overlay shell — in an ordinary browser there is no channel and
// the page is simply a page. qwebchannel.js is injected by the shell, so
// nothing is served for it here.
if (window.qt?.webChannelTransport) {
  new QWebChannel(window.qt.webChannelTransport, (channel) => {
    shell = channel.objects.shell;
    shell.geometry.connect(onGeometry);
    shell.userToggled.connect(toggleGhost);
    // The status event that carried it has usually already been and gone.
    if (windowName) shell.setWindowName(windowName);
    report();
  });
}
