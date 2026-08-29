// The overlay strip's whole client: draw the newest line, and define the word
// that is clicked in it.
//
// Only the newest line is ever drawn. The few before it are asked for through
// the stream's `backlog` so the explain button has context from the moment the
// overlay opens; on a dropped connection EventSource reconnects with
// `Last-Event-ID`, so it resumes after the line it drew.
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
// running beside Anki. kotodex-server proxies it: that server binds loopback
// and sends no CORS headers, so neither this page nor a phone reading the
// overlay can ask it directly.
//
// Three actions on a word, and only one of them opens the popup: left-click
// asks what it means, the back side button judges it known or unknown, the
// forward one mines it. Splitting them that way is what keeps the lookup count
// honest — see `SIDE_ACTIONS`. The wheel over that word pages the popup's
// dictionaries, which is not a fourth action: the popup is already open, so
// nothing is looked up and nothing is written.

import { createPopup } from "/shared/popup.js";
import { parseMarkdown } from "/shared/markdown.js";
import { NO_KEY, streamExplain } from "/shared/explain.js";
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
const infoEl = document.getElementById("info");

// What is wrong, one line per thing. Two faults are two lines in the one box
// rather than one of them winning: a shut Anki and a dead capture are unrelated
// problems, and ranking them means the loser is invisible until the winner is
// fixed.
//
// Standing faults are keyed and rewritten in place — the poll that reports one
// repeats every couple of seconds — while a one-off (a mine Anki refused) is a
// line with a timer on it, since nothing is going to come back and clear it.
const faults = new Map();
// text -> the timer that clears it, so the same message twice is one line with
// its clock restarted rather than two.
const transient = new Map();

function drawWarnings() {
  warnEl.replaceChildren(
    ...[...faults.values()].map(({ text, act }) => {
      // A fault the overlay can fix from here *is* the control that fixes it.
      // Saying where a setting lives and leaving the reader to find it is the
      // same sentence with a walk in the middle.
      const line = document.createElement(act ? "button" : "div");
      line.textContent = text;
      if (act) line.addEventListener("click", act);
      return line;
    }),
    ...[...transient.keys()].map((text) => {
      const line = document.createElement("div");
      line.textContent = text;
      return line;
    }),
  );
}

function setFault(key, text, act) {
  if ((faults.get(key)?.text ?? "") === (text ?? "")) return;
  if (text) faults.set(key, { text, act });
  else faults.delete(key);
  drawWarnings();
}

function warn(text, holdMs = 6000) {
  clearTimeout(transient.get(text));
  transient.set(
    text,
    setTimeout(() => {
      transient.delete(text);
      drawWarnings();
    }, holdMs),
  );
  drawWarnings();
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
// What is being read, as the status event last reported it. Empty is a state a
// reader can be in and the one worth reporting: every line captured then is
// stamped with no title.
let currentWork = "";
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
// Which model answers. `hasKey` and never the key: the server does not return
// one, so the box shows whether something is stored rather than what.
let llm = { provider: "anthropic", baseUrl: "", model: "", hasKey: false };
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
    llm = {
      provider: s.llm_provider || "anthropic",
      baseUrl: s.llm_base_url || "",
      model: s.llm_model || "",
      hasKey: s.llm_has_key === true,
      keyFromEnv: s.llm_key_from_env === true,
    };
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
  const { capture, paused, vn_window, work } = JSON.parse(e.data);
// What each capture state says to a reader who is looking at the game, not at
// a log. `live` never reaches here.
//
// Read off the status event and nothing else: whether a source is attached is
// a fact about this second, while `capabilities` is fetched once as the page
// opens — and the overlay comes up beside the daemon it reports on, so that
// answer is taken before the source has connected and then never changes.
// `capture === "live"` already means one is attached.
/** How long after the pause flag moves to stop reporting a missing source.
 *
 *  Resuming is not instant, and nothing here can make it so: three independent
 *  two-second waits sit between the click and the answer. The logger is parked
 *  in its pause poll (`PAUSE_POLL_SECS`), the flag it reads there may be up to
 *  `SETTINGS_TTL` old, and this surface is told by a status event republished
 *  every two seconds. Six seconds is that worst case.
 *
 *  Reported as nothing at all rather than as a fault: for this window the
 *  answer is not "no source", it is "not known yet", and a fault raised for the
 *  app doing exactly what it was told is how a reader learns to ignore the
 *  box. A Textractor that really is down is still reported, six seconds later. */
const RESUME_SETTLE_MS = 6000;
let pauseMovedAt = 0;
let wasPaused = null;

const CAPTURE_FAULT = {
  unhooked: "no line source — is Textractor running with its WebSocket plugin?",
  down: "capture is not running — start Kotodex, or `kotodex-capture restart`",
  stalled: "lines are not reaching the ledger — is kotodex-server up?",
};

  // Nothing while paused: the reader chose it, the pause button already says
  // so, and every fault here is about lines not arriving — which is the point
  // of pausing, not a problem with it.
  if (wasPaused !== null && wasPaused !== paused) pauseMovedAt = Date.now();
  wasPaused = paused;
  const settling = Date.now() - pauseMovedAt < RESUME_SETTLE_MS;
  setFault(
    "capture",
    paused || settling || capture === "live"
      ? ""
      : (CAPTURE_FAULT[capture] ?? capture),
  );
  // A chosen state, said plainly and in its own box: it is not a fault, and
  // colouring it like one would be the app arguing with the reader about a
  // button they just pressed.
  infoEl.textContent = paused
    ? "⏸ Capture paused — no lines are being recorded."
    : "";
  // **Not suppressed while paused**, unlike the capture fault above. That rule
  // is about lines not arriving, which is what pausing is *for*; these two are
  // about the overlay being set up wrong, and a pause does not make an
  // unconfigured overlay correct — a reader pauses in order to go and fix one.
  //
  // Two different absences, and the second is not a milder version of the
  // first. With no work at all nothing read is counted anywhere, and this
  // surface cannot fix it — picking a work is a VNDB search — so that line
  // opens the dashboard instead of the panel.
  setFault(
    "work",
    work
      ? ""
      : "nothing is being read. Click here to pick a work",
    openDashboard,
  );
  setFault(
    "window",
    !work || vn_window
      ? ""
      : "the overlay is not attached to the game — click here to pick its window",
    openWindowSettings,
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
  if (name !== windowName || (work ?? "") !== currentWork) {
    windowName = name;
    currentWork = work ?? "";
    shell?.setWindowName(windowName);
    showWindowSetting();
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

/** One line, drawn the way the live line is drawn.
 *
 * Shared with the scrollback, which is the reason it is a function rather than
 * the body of `draw`. A line the reader scrolled back to is the same line it
 * was a minute ago, and a second implementation would drift from this one
 * exactly where it matters — what a word is worth knowing about. */
function renderLine(row) {
  const text = row.text;
  const ruby = row.ruby ?? [];
  const frag = document.createDocumentFragment();
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
      // No frequency dictionary means no answer to "is this word common", so
      // nothing is underlined rather than everything reading as rare.
      const common =
        type.markCommon &&
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

// `append`: whether the line is new to the history. False when it is being put
// back on screen because the one after it was cleared — its row is already in
// the panel, and appending would show it twice.
function draw(incoming, again = false, append = true) {
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
  // Whether the panel is *populated*, not whether it is open: it seeds only
  // when it has no rows at all, so a line read with the panel shut would
  // otherwise be missing from the history for good.
  //
  // Nothing before the first open, so that the seed and its page back are what
  // build the panel — appending into an empty one would leave it holding this
  // session with no way to reach anything older.
  if (append && scrollbackLinesEl.children.length) appendScrollback(line);
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
document.addEventListener(
  "pointerdown",
  (e) => {
    if (popupEl.hidden) return;
    if (popupEl.contains(e.target)) return;
    if (e.target.closest?.(".w")) return;
    closePopup();
  },
  true,
);

// A click on anything that is not this surface — the game, a browser, the
// desktop — never reaches the handler above: the input region ends at what is
// drawn, so the compositor hands that click straight to the window underneath
// and the page hears nothing about it. What it does hear is losing the
// keyboard, which is the same event seen from this side, and it covers every
// window rather than only the one whose new line happened to redraw the strip.
window.addEventListener("blur", () => closePopup());

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
 * mining one, is not that, so neither goes through the popup: reaching a button
 * there would record a lookup that never happened.
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
const gripEls = [...document.getElementsByClassName("size-grip")];
// Which way each corner sits, as `[down, right]`. Dragging a corner away from
// the line is what grows the type, so a top grip counts a drag upwards as bigger
// — the same gesture as a window's own corners.
const CORNERS = {
  tl: [false, false],
  tr: [false, true],
  bl: [true, false],
  br: [true, true],
};
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
  placeGrip();
  report();
}

/** The resize grip, in the line's own bottom-right corner.
 *
 * Positioned here rather than in CSS because #line is `width: fit-content`
 * inside a strip that is not: the corner it has to sit on is wherever the text
 * happened to end. Held a little inside that corner so it stays within the hit
 * region, which is measured off the line. */
function placeGrip() {
  const line = lineEl.getBoundingClientRect();
  const box = boxEl.getBoundingClientRect();
  for (const grip of gripEls) {
    const [down, right] = CORNERS[grip.dataset.corner];
    const x = right ? line.right - box.left - 15 : line.left - box.left + 2;
    const y = down ? line.bottom - box.top - 15 : line.top - box.top + 2;
    grip.style.left = `${x}px`;
    grip.style.top = `${y}px`;
  }
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

// Resizing the type by dragging it, which writes `scale` — the same setting the
// panel's slider does, so the two agree and the readout follows the drag.
//
// Two ways in: the grip, and shift held anywhere on the line. The slider's own
// bounds, because a drag past them is a value the panel could not show.
const SCALE_MIN = 0.6;
const SCALE_MAX = 2;
let sizing = null;

/** One corner of a rectangle, named the way `CORNERS` names it. */
function cornerAt(rect, [down, right]) {
  return { x: right ? rect.right : rect.left, y: down ? rect.bottom : rect.top };
}

function beginSize(e, el, corner) {
  const rect = lineEl.getBoundingClientRect();
  const [down, right] = corner;
  const opposite = cornerAt(rect, [!down, !right]);
  // The gesture runs along the box's own diagonal, so a drag has to be measured
  // along it rather than down the screen: how far the pointer has gone *out
  // from the opposite corner*, as a fraction of the diagonal, is how much
  // bigger the box should be. The direction and the length are both taken once,
  // from the box as it was grabbed — measuring them again each move would feed
  // the box's new size back into the gesture driving it.
  const diagonal = Math.hypot(rect.width, rect.height) || 1;
  sizing = {
    id: e.pointerId,
    x: e.clientX,
    y: e.clientY,
    from: type.scale,
    opposite,
    hold: [!down, !right],
    ux: ((right ? 1 : -1) * rect.width) / diagonal,
    uy: ((down ? 1 : -1) * rect.height) / diagonal,
    diagonal,
  };
  el.setPointerCapture(e.pointerId);
  for (const grip of gripEls) grip.classList.add("on");
}

function sizeTo(e) {
  const along =
    (e.clientX - sizing.x) * sizing.ux + (e.clientY - sizing.y) * sizing.uy;
  // Multiplicative: a drag that adds a fixed step per pixel races at small
  // sizes and barely moves at large ones.
  const next = sizing.from * (1 + along / sizing.diagonal);
  const held = Math.min(SCALE_MAX, Math.max(SCALE_MIN, next));
  type = { ...type, scale: Math.round(held * 100) / 100 };
  applyType();
  // The corner opposite the one being dragged is the fixed point, the way any
  // resize handle works. Nothing in the CSS holds it: the line is placed by its
  // first glyph, so growing the type walks every other corner away. Putting it
  // back is a correction to the same drag offset the move gesture writes.
  const now = cornerAt(lineEl.getBoundingClientRect(), sizing.hold);
  const at = offsetPx();
  moveTo(at.x + sizing.opposite.x - now.x, at.y + sizing.opposite.y - now.y);
  // Placed off the line box, which is growing out from under it.
  if (popup.anchor()) place(popup.anchor());
}

function endSize(e, el) {
  el.releasePointerCapture(e.pointerId);
  for (const grip of gripEls) grip.classList.remove("on");
  sizing = null;
  // The re-anchoring above moved the box, which is the same stored placement
  // the move drag writes.
  if (game) localStorage.setItem(ALIGNED_PLACE, JSON.stringify(alignedOffset));
  else localStorage.setItem(PLACE, JSON.stringify(offset));
  // The click that ends a drag is not a click on the overlay.
  document.addEventListener("click", (c) => c.stopPropagation(), { capture: true, once: true });
}

for (const grip of gripEls) {
  grip.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    // The grip sits inside the line, so the move drag would start under it.
    e.stopPropagation();
    beginSize(e, grip, CORNERS[grip.dataset.corner]);
  });

  grip.addEventListener("pointermove", (e) => {
    if (sizing && e.pointerId === sizing.id) sizeTo(e);
  });

  for (const event of ["pointerup", "pointercancel"]) {
    grip.addEventListener(event, (e) => {
      if (sizing && e.pointerId === sizing.id) endSize(e, grip);
    });
  }
}

lineEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  // Shift takes the words as well as the gaps between them: a line is mostly
  // words, and the move drag's own grab area is too little to resize from.
  if (e.shiftKey) {
    beginSize(e, lineEl, CORNERS.br);
    return;
  }
  if (e.target.closest(".w")) return;
  // Grab from wherever the box currently sits, which aligned to the game is
  // the fraction scaled up — not the free-floating offset, which is then zero.
  const at = offsetPx();
  drag = { id: e.pointerId, x: e.clientX - at.x, y: e.clientY - at.y, moved: false };
  lineEl.setPointerCapture(e.pointerId);
  lineEl.classList.add("moving");
});

lineEl.addEventListener("pointermove", (e) => {
  if (sizing && e.pointerId === sizing.id) return sizeTo(e);
  if (!drag || e.pointerId !== drag.id) return;
  drag.moved = true;
  moveTo(e.clientX - drag.x, e.clientY - drag.y);
  // The popup is placed off the line box, so it has to be re-placed as the box
  // moves out from under it.
  if (popup.anchor()) place(popup.anchor());
});

for (const type of ["pointerup", "pointercancel"]) {
  lineEl.addEventListener(type, (e) => {
    if (sizing && e.pointerId === sizing.id) return endSize(e, lineEl);
    if (!drag || e.pointerId !== drag.id) return;
    lineEl.releasePointerCapture(e.pointerId);
    lineEl.classList.remove("moving");
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
// The key names the anchor the offset was stored against: the widget hangs off
// the top edge, and the same offset against any other anchor places it
// somewhere else entirely.
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
 *  against moves — the widget hangs off the game's corner, and the game
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
// the pointer captured does not always lift on the button it went down on. A
// drag that actually moved is not a click.
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
  // The warning belongs beside the controls for the thing it is about, so a shut
  // bar takes it with it — on its own over the art it is just a chip the reader
  // cannot act on.
  explainBoxEl.toggleAttribute("data-shut", !open);
  if (!open) closeBarPanels(null);
  report();
}

explainBtnEl.addEventListener("click", explainLine);

// The widget is its own surface: a click on it must not reach the document
// handler that closes the popup, and must not reach the VN either.
explainBoxEl.addEventListener("click", (e) => e.stopPropagation());

// The bar is not the popup's surface, so pressing a button on it is done with
// the open word. The line above is what stops the document handler doing this,
// and the buttons' own handlers have already run by the time this does — which
// is what leaves `explainLine` the anchor it takes its focus from.
buttonsEl.addEventListener("click", () => closePopup());

// Reading it is what it is for, so it stays until dismissed — and a click
// anywhere on it dismisses, rather than a ✕ to aim at over a game.
explainPanelEl.addEventListener("click", hideExplain);

// The button row's tooltip is drawn by the page — see `[data-tip]` in
// overlay.html — so `title` must stay unset or the native one draws too.
const tip = (el, text) => el.setAttribute("data-tip", text);

/* Earlier lines, over the whole surface.
 *
 * A page of history is fetched only when the top is reached, so opening it
 * costs one request and scrolling back a thousand lines costs one per page.
 * The rows are built by `renderLine`, so every word in them clicks, judges and
 * mines exactly as the live line's do, and a lookup from here is a lookup: a
 * word met three lines ago is the commonest thing to want to look up.
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
 *  every scrollable ancestor, and the panel is inside the surface. */
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
  // #box, not #line. #line is `width: fit-content`, so measuring it gives the
  // width of whatever text happens to be showing rather than the column the
  // line is set in, which is what the character count means.
  const width =
    boxEl.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
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

// The same write `#read`'s "✕ clear last" makes: the line stops counting toward
// anything derived, without being deleted. The id comes from the line on screen
// rather than the server picking "the last one", so a line hooked mid-click is
// not the one that goes.
const clearBtnEl = document.getElementById("clear-btn");
let clearing = false;

async function clearLast() {
  if (clearing || !line || line.id == null) return;
  const dropped = line;
  clearing = true;
  clearBtnEl.disabled = true;
  try {
    const res = await fetch("/api/lines/discard", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ids: [dropped.id] }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const { ids = [] } = await res.json();
    if (!ids.length) return;

    scrollbackLinesEl.querySelector(`.sb[data-id="${dropped.id}"]`)?.remove();
    // The explain context is the last few lines *read*, and this one no longer
    // counts as read.
    const at = recent.lastIndexOf(dropped.text);
    if (at !== -1) recent.splice(at, 1);

    // Back to the line before it, which is what the reader is now looking at.
    const older = await fetch(`/api/lines/before?before=${dropped.id}&limit=1`)
      .then((r) => r.json())
      .catch(() => ({}));
    const prev = (older.lines ?? [])[0];
    if (prev) {
      draw(prev, true, false);
      scrollbackLinesEl
        .querySelector(`.sb[data-id="${prev.id}"]`)
        ?.classList.add("current");
    } else {
      line = null;
      lineEl.replaceChildren();
    }
    markSessions();
    countScrollback();
  } catch (err) {
    warn(`could not clear the line — ${err.message}`, 6000);
  } finally {
    clearing = false;
    clearBtnEl.disabled = false;
  }
}
clearBtnEl.addEventListener("click", clearLast);

// The line off the screen without stopping the overlay: a scene worth looking
// at is worth looking at whole. The stream keeps running, so the line that
// comes back is whatever is current rather than the one that was showing.
const hideBtnEl = document.getElementById("hide-btn");
hideBtnEl.addEventListener("click", () => {
  boxEl.hidden = !boxEl.hidden;
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
  // An icon, not a phrase: it sits in the row of square buttons over the game,
  // and the tooltip is where the sentence goes. Which of the button's two icons
  // shows is CSS off this class.
  pauseBtnEl.classList.toggle("paused", paused);
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

// The dashboard, in a browser. Never in this view: the surface *is* the page,
// so navigating it away would take the overlay with it — under the shell the
// URL goes out to the desktop's browser, and in an ordinary browser a tab is a
// tab.
document.getElementById("stats-btn").addEventListener("click", () => {
  const url = `${location.origin}/`;
  if (shell) shell.openUrl(url);
  else window.open(url, "_blank");
});

// The overlay's settings, in three tabs: how the line is set and sized, what
// is marked on it, and where its lines come from.
//
// Most of them are stored in the browser, because they are about this screen —
// a phone reading the same overlay wants its own — and applied as CSS
// variables, so the aligned placement over the game's own text keeps working
// off them. The two that every reading surface has to agree on — whether a
// status is painted at all, and the rank an unknown word counts as common
// under — are read from and written back to kotodex-server.
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
  // a phone, and the size slider is already the way to change it.
  mobileScale: 1.75,
  tint: 0.85,
  markNew: true,
  markSeen: true,
  markUnknown: true,
  markCommon: false,
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
  markCommon: { id: "set-mark-common", check: true, repaints: true },
};

let type = { ...TYPE_DEFAULTS };
try {
  const stored = JSON.parse(localStorage.getItem(TYPE) ?? "{}");
  // Key by key, and only where the stored value is still the same kind of
  // thing. The shell keeps localStorage across releases, so a setting whose
  // shape has changed arrives as the old kind and would throw the moment its
  // readout is formatted — taking the rest of this file's setup with it: no
  // input region, and an overlay nothing can be clicked on.
  for (const [key, value] of Object.entries(stored)) {
    if (key in TYPE_DEFAULTS && typeof value === typeof TYPE_DEFAULTS[key]) type[key] = value;
  }
} catch {
  // Nothing stored, or not JSON at all. Draw the line as measured.
}

const settingsBtnEl = document.getElementById("settings-btn");
const quitBtnEl = document.getElementById("quit-btn");
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
  // Nothing chosen and nothing launched with: the stylesheet's own stack, which
  // is the only one that names a face on every platform. A default named here
  // would pin one family and leave Windows on generic `sans-serif`.
  const chosen = type.font || font;
  if (chosen) root.setProperty("--line-font", `"${chosen}", sans-serif`);
  else root.removeProperty("--line-font");
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
  // Rounded: a stored fraction would throw in `Array`.
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

/** Open the panel on one tab. Also how the two faults that can be fixed from
 *  here send a reader to the box that fixes them. */
function showTab(name) {
  settingsPanelEl.hidden = false;
  settingsBtnEl.classList.remove("off");
  for (const btn of tabBtnEls) btn.classList.toggle("on", btn.value === name);
  for (const body of tabBodyEls) body.hidden = body.dataset.tab !== name;
  report();
}

for (const btn of tabBtnEls) {
  btn.addEventListener("click", () => {
    showTab(btn.value);
    // What is open changes while the panel is shut, so the list is taken when
    // the tab holding it is reached rather than kept up to date.
    if (btn.value === "source") loadWindows();
  });
}

settingsBtnEl.addEventListener("click", () => {
  settingsPanelEl.hidden = !settingsPanelEl.hidden;
  settingsBtnEl.classList.toggle("off", settingsPanelEl.hidden);
  report();
  if (!settingsPanelEl.hidden) loadWindows();
});

applyType();
// The placement clamp and the grips are both measured off the line, and the
// stored type settings only reach it here — so both have to be taken again at
// the size the line will actually be.
apply();

// The two settings kotodex-server owns, written back as they are changed: `#read`
// underlines by the same rank and paints by the same flag, and a switch that
// only moved this page would make the two surfaces disagree about the same
// word.
const statusInputEl = document.getElementById("set-status");
const commonInputEl = document.getElementById("set-common");

const sourceBoxEl = document.getElementById("set-line-source");
const sourceRowEls = [...sourceBoxEl.children];
const wsUrlEl = document.getElementById("set-ws-url");
const sourceNoteEl = document.getElementById("source-note");

const llmProviderRowEls = [...document.getElementById("set-llm-provider").children];
const llmBaseUrlEl = document.getElementById("set-llm-base-url");
const llmModelEl = document.getElementById("set-llm-model");
const llmKeyEl = document.getElementById("set-llm-key");
const llmKeySaveEl = document.getElementById("llm-key-save");
const llmKeyNoteEl = document.getElementById("llm-key-note");
const llmNoteEl = document.getElementById("llm-note");

/** What each service wants in the two boxes under it. */
const LLM_SERVICES = {
  anthropic: {
    baseUrl: "https://api.anthropic.com",
    note: "Claude, from console.anthropic.com. Leave the address alone unless you are proxying it.",
  },
  openai: {
    baseUrl: "https://api.openai.com/v1",
    note: "Anything speaking the OpenAI chat API: OpenAI, OpenRouter, DeepSeek, Gemini, or a local model. Include /v1 in the address, and name a model.",
  },
};

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
      ? "Anything copied is treated as a line, including text copied for a lookup."
      : "Textractor's WebSocket plugin. The port is the one set in Textractor.";
  showLlmSettings();
}

function showLlmSettings() {
  const service = LLM_SERVICES[llm.provider] ?? LLM_SERVICES.anthropic;
  for (const row of llmProviderRowEls) {
    row.classList.toggle("on", row.dataset.provider === llm.provider);
  }
  if (document.activeElement !== llmBaseUrlEl) {
    // The service's own address as the placeholder rather than the value: stored
    // empty means "whatever this service uses", and filling the box in would
    // save a URL the reader never chose.
    llmBaseUrlEl.value = llm.baseUrl;
    llmBaseUrlEl.placeholder = service.baseUrl;
  }
  if (document.activeElement !== llmModelEl) llmModelEl.value = llm.model;
  llmNoteEl.textContent = service.note;
  llmKeySaveEl.disabled = llmKeyEl.value.trim() === "" && !llm.hasKey;
  if (!llmKeyNoteEl.dataset.said) {
    llmKeyNoteEl.textContent = llm.hasKey
      ? "A key is stored. Paste another to replace it, or save an empty box to remove it."
      : llm.keyFromEnv
        ? "KOTODEX_ANTHROPIC_API_KEY is what answers. Paste a key here to use that one instead."
        : "Needed for explaining a line, and for the short gloss on a mined card. Everything else works without one.";
  }
}

/** Store the key and say whether it actually answered. */
async function saveLlmKey() {
  llmKeySaveEl.disabled = true;
  llmKeyNoteEl.dataset.said = "1";
  llmKeyNoteEl.textContent = "checking…";
  try {
    const res = await fetch("/api/settings/llm-key", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ api_key: llmKeyEl.value }),
    });
    const out = await res.json();
    llm = { ...llm, hasKey: llmKeyEl.value.trim() !== "" };
    llmKeyEl.value = "";
    llmKeyNoteEl.textContent = out.detail || (out.ok ? "saved" : "could not be checked");
    llmKeyNoteEl.classList.toggle("err", out.ok !== true);
    // Redrawn from the server's own answer, so the row the reader reads next is
    // the row the reader surfaces will read.
    if (out.ok) refreshCapabilities();
  } catch (e) {
    llmKeyNoteEl.textContent = String(e.message || e);
    llmKeyNoteEl.classList.add("err");
  } finally {
    llmKeySaveEl.disabled = llmKeyEl.value.trim() === "" && !llm.hasKey;
  }
}

function refreshCapabilities() {
  fetch("/api/reader/state")
    .then((r) => r.json())
    .then((s) => {
      caps = s.capabilities ?? {};
      applyCapabilities();
    })
    .catch(() => {});
}

llmKeySaveEl.addEventListener("click", saveLlmKey);
llmKeyEl.addEventListener("input", () => {
  llmKeySaveEl.disabled = llmKeyEl.value.trim() === "" && !llm.hasKey;
});
llmKeyEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter") saveLlmKey();
});

for (const row of llmProviderRowEls) {
  row.addEventListener("click", () => {
    llm = { ...llm, provider: row.dataset.provider };
    showLlmSettings();
    saveSetting("llm_provider", llm.provider);
  });
}

// On change rather than on input, for the same reason as the WebSocket address:
// a half-typed URL is not one worth storing.
llmBaseUrlEl.addEventListener("change", () => {
  llm = { ...llm, baseUrl: llmBaseUrlEl.value.trim() };
  saveSetting("llm_base_url", llm.baseUrl);
});
llmModelEl.addEventListener("change", () => {
  llm = { ...llm, model: llmModelEl.value.trim() };
  saveSetting("llm_model", llm.model);
});

/** Open ⚙ on the AI tab with the key box focused. Where the explain button sends
 *  a reader who has no key, so the answer to pressing it is the thing that turns
 *  it on rather than a message about a variable. */
function openAiSettings() {
  showTab("ai");
  llmKeyEl.focus();
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

// Which window is the game. The same column the dashboard's work editor writes
// and `vn-capture.sh` reads — put here because the overlay is where its absence
// is noticed: nothing else on this surface reports that a card's screenshot is
// about to grab the whole screen with the overlay on it.
//
// A list of what is open rather than a box to type a title into. The reader is
// picking one of a handful of running programs, and typing is the answer to a
// question nobody has: the game's own title is what the tracker wants and the
// game is the thing that knows it.
//
// Written through `PUT /api/vn/window` rather than by work id. This page has no
// library and no id; what it has is the work being read, which is the one the
// window belongs to.
const vnWindowEl = document.getElementById("set-vn-window");
const vnWindowNoteEl = document.getElementById("vn-window-note");

let openWindows = [];
let focusedWindow = null;

/** What is open, asked when the panel is opened rather than held.
 *
 * The answer costs a round trip to the window manager and goes stale the moment
 * the game is started or quit, so it is worth taking exactly when someone is
 * looking at the list. */
async function loadWindows() {
  try {
    const r = await (await fetch("/api/vn/windows")).json();
    openWindows = r.windows ?? [];
    focusedWindow = r.focused ?? null;
  } catch {
    openWindows = [];
    focusedWindow = null;
  }
  showWindowSetting();
}

/** One pickable window. Rows, **never a `<select>` or a `<datalist>`** — see
 *  the chips comment in overlay.html: both open a native popup window, and a
 *  layer surface has none to open one in, so the list simply never appears.
 *  Every other choice on this panel is drawn the same way for the same reason. */
function windowRow(value, text, on) {
  const el = document.createElement("div");
  el.className = on ? "row on" : "row";
  el.dataset.window = value;
  el.textContent = text;
  return el;
}

function showWindowSetting(name = windowName) {
  // Nothing to pick from without a work to pick it for: the note carries the
  // reason and the way out, which is a better empty list than a dead one.
  if (!currentWork) {
    vnWindowEl.replaceChildren();
  } else {
    const rows = [windowRow("", "— not set —", !name)];
    // Kept in the list even when the game is not running: it is the setting's
    // value, and dropping it would make closing the game look like losing it.
    if (name && !openWindows.includes(name)) {
      rows.push(windowRow(name, `${name} (not open)`, true));
    }
    for (const w of openWindows) {
      // The window in front is the answer on a good day — the overlay never
      // takes focus, so the game still has it. Marked in the list rather than
      // offered as a button beside it, which would be a second control for one
      // choice. KDE under Wayland answers nothing here, so it is a hint and
      // never the mechanism.
      rows.push(windowRow(w, w === focusedWindow ? `${w} — in front` : w, w === name));
    }
    vnWindowEl.replaceChildren(...rows);
  }
  showWindowNote(name);
}

/** The note under the list, with the way out of the no-work state built into
 *  it. A sentence telling the reader to go and pick a work, on a panel that
 *  hides every warning while it is open, would be the only dead end left. */
function showWindowNote(name) {
  const err = !currentWork || !name;
  vnWindowNoteEl.classList.toggle("err", err);
  if (!currentWork) {
    const link = document.createElement("button");
    link.className = "note-link";
    link.textContent = "Pick what you are reading";
    link.addEventListener("click", openDashboard);
    vnWindowNoteEl.replaceChildren(
      "Nothing is being read, so there is no work to attach. ",
      link,
    );
    return;
  }
  vnWindowNoteEl.textContent = windowNote(name);
}

/** What the list is for, and whether the current answer still names a window.
 *
 *  Attaching the overlay to the game is the thing worth pressing for: it is what
 *  lets the line be laid over the game's own text, follow it as it moves or goes
 *  fullscreen, and be screenshotted onto a card. Said once, here. */
function windowNote(name) {
  if (!name) {
    return "Pick the game's window so the overlay can follow it.";
  }
  if (!openWindows.length) return "";
  return openWindows.includes(name)
    ? "Attached."
    : "Not open right now. Pick it again once the game is running.";
}

async function saveWindow(name) {
  try {
    const res = await fetch("/api/vn/window", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ window: name.trim() }),
    });
    if (!res.ok) {
      vnWindowNoteEl.textContent = (await res.text()) || "could not be saved";
      vnWindowNoteEl.classList.add("err");
      return;
    }
  } catch (e) {
    vnWindowNoteEl.textContent = String(e.message || e);
    vnWindowNoteEl.classList.add("err");
    return;
  }
  // The status event carries it back within two seconds and the shell is told
  // then, so nothing here keeps a second copy of which window is the game.
  showWindowSetting(name);
}

// Delegated: the rows are rebuilt whenever the list or the setting changes.
vnWindowEl.addEventListener("click", (e) => {
  const row = e.target.closest(".row");
  if (row) saveWindow(row.dataset.window);
});

/** Open ⚙ on the Source tab with the list of windows fresh. Where the "no
 *  window name" warning sends a reader, so pressing it is the thing that fixes
 *  it rather than a sentence about where to look. */
function openWindowSettings() {
  showTab("source");
  loadWindows();
  vnWindowEl.focus();
}

/** The dashboard, in the desktop's browser.
 *
 *  The one thing this surface cannot do for itself: picking a work is a VNDB
 *  search, and the overlay is a strip over a game. `#today` asks the question
 *  with the box focused whenever nothing is being read, so the link lands on
 *  it without needing to say so. */
function openDashboard() {
  const url = new URL("/", location.href).href;
  if (shell) shell.openUrl(url);
  else window.open(url, "_blank");
}

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
    // No key is not an error to read, it is a box to fill in.
    if (err.message === NO_KEY) {
      hideExplain();
      openAiSettings();
    } else {
      showExplain(err.message, true);
    }
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

function hideExplain() {
  explainPanelEl.hidden = true;
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
  //
  // Pinned by its bottom edge when it goes above, so that a definition which
  // changes height — paging to a longer dictionary — grows away from the line
  // rather than down over it. That is worth more than the clamping a `top`
  // anchor would allow: `place` runs again once the content is in, so the
  // choice above is made against the real height rather than a placeholder's.
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
  const { note_id, error } = await res.json().catch(() => ({}));
  // Anki refuses in its own words and with a 200, so this is the only place the
  // reason exists. Held on screen: a mine that quietly does nothing is
  // indistinguishable from a click that missed, and the reason is usually
  // something only the reader can fix — the wrong profile is open, the note
  // type was renamed.
  if (!note_id) warn(error ? `Anki: ${error}` : "Anki added no card", 12000);
  // Ask again straight away: a mine is the one moment a shut Anki costs
  // something, and the poll may be most of its interval away.
  if (!note_id) checkAnki();
  return note_id;
}

// Whether Anki is answering where a card would be added. Polled, because
// nothing else asks until a mine does, and a failing mine is the worst moment
// to find out that Anki is shut.
const ANKI_POLL_MS = 20_000;

async function checkAnki() {
  try {
    const { up, mining_used } = await (await fetch("/api/anki/up")).json();
    // Silent on an install that has never mined a card: Anki is optional, and a
    // standing fault for a part the reader has not asked for is what teaches
    // them to ignore the box. A mine that fails still says so, once.
    setFault(
      "anki",
      up || !mining_used
        ? ""
        : "no Anki — start it with the AnkiConnect add-on; required for mining",
    );
  } catch {
    // kotodex-server itself did not answer, which says nothing about Anki. The
    // capture fault is the line that reports that.
  }
}

checkAnki();
setInterval(checkAnki, ANKI_POLL_MS);

/** Tell the shell every rectangle on this page that should take a click.
 *
 * It hands them to `wl_surface.set_input_region`, so the overlay is clickable
 * exactly where it has drawn something and the VN gets everything else —
 * clicking on to the next line never touches the overlay at all. CSS pixels and
 * window pixels are the same thing here: the view fills the surface.
 *
 * Pushed the moment the layout changes, not polled: a region that lags what is
 * on screen by a tick sends a click aimed at a popup the compositor does not
 * know about yet to the VN, which advances the line and closes that popup.
 */
let reportQueued = false;
let reportedHits = null;
let keyboardWanted = null;

function setKeyboard(want) {
  if (want === keyboardWanted) return;
  keyboardWanted = want;
  if (shell.setKeyboard) shell.setKeyboard(want);
  else console.error("shell.setKeyboard is missing");
}

function report() {
  if (!shell || reportQueued) return;
  reportQueued = true;
  setTimeout(sendHits, 0);
}

function sendHits() {
  reportQueued = false;
  if (!shell) return;
  setKeyboard(!settingsPanelEl.hidden);
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
  // `#line:empty` is display:none, so its rectangle is at the origin and would
  // put a padding-sized hit region in the corner of the screen.
  if (!boxEl.hidden && lineEl.firstChild) rects.push(lineEl.getBoundingClientRect());
  if (!popupEl.hidden) rects.push(popupEl.getBoundingClientRect());
  // Flat `x, y, w, h, ...` rather than nested: an array of arrays reaches Qt
  // as opaque QJSValues, while an array of plain numbers converts cleanly.
  //
  // A few pixels of slack, so a click on the very edge of the backdrop is
  // still caught and the region survives a subpixel reflow.
  const hits = rects.flatMap((r) => [r.left - 4, r.top - 4, r.width + 8, r.height + 8]);
  const key = hits.join();
  if (key === reportedHits) return;
  reportedHits = key;
  shell.setHits(hits);
}

// Anything that moves either box without going through `report` itself: the web
// font landing, a long line wrapping, a definition arriving and growing the
// popup upwards.
const watch = new ResizeObserver(() => {
  placeGrip();
  report();
});
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

ghostInputEl.addEventListener("change", () => setGhost(ghostInputEl.checked));
applyGhost();


// Only under the overlay shell — in an ordinary browser there is no channel and
// the page is simply a page. qwebchannel.js is injected by the shell, so
// nothing is served for it here.
if (window.qt?.webChannelTransport) {
  new QWebChannel(window.qt.webChannelTransport, (channel) => {
    shell = channel.objects.shell;
    shell.geometry.connect(onGeometry);
    if (shell.dismissed) shell.dismissed.connect(() => closePopup());
    else console.error("shell.dismissed is missing");
    // Only under the shell: opened in a browser the page has no process to end.
    // This quits Kotodex, not just this window — the shell turns it into that,
    // and on a desktop with no system tray it is the only way out.
    quitBtnEl.hidden = false;
    quitBtnEl.addEventListener("click", () => shell.quit());
    // The status event that carried it has usually already been and gone.
    if (windowName) shell.setWindowName(windowName);
    report();
  });
}
