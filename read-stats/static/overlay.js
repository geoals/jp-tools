// The overlay strip's whole client: draw the newest line, and define the word
// that is clicked in it.
//
// `backlog=1` is what the stream's query parameter exists for — this caller
// wants a short feed, and a feed of one is the shortest. On a dropped
// connection EventSource reconnects with `Last-Event-ID`, so it resumes after
// the line it drew rather than replaying anything.
//
// Segmentation is not asked for separately: the line event already carries a
// span per word, each with the `(headword, reading)` the ledger keys on. The
// popup asks about that pair, so 振っ is defined as 振る.
//
// Three actions on a word, and only one of them opens the popup: left-click
// asks what it means, the back side button judges it known or unknown, the
// forward one mines it. Splitting them that way is what keeps the lookup count
// honest — see `SIDE_ACTIONS`. The wheel over that word pages the popup's
// dictionaries, which is not a fourth action: the popup is already open, so
// nothing is looked up and nothing is written.

const params = new URLSearchParams(location.search);
const root = document.documentElement.style;
root.setProperty("--backdrop", `rgba(0, 0, 0, ${params.get("bg") ?? "0.55"})`);
root.setProperty("--strip", `${params.get("h") ?? "300"}px`);
// Only the line, never the popup: the popup is a dictionary, and reading it in
// a display face the game text is being tried in makes both harder to judge.
const font = params.get("font");
if (font) root.setProperty("--line-font", `"${font}", sans-serif`);

const lineEl = document.getElementById("line");
const warnEl = document.getElementById("warn");
const popupEl = document.getElementById("popup");

let openWord = null;
let line = null;
// The overlay shell, once its channel is up. Null in an ordinary browser.
let shell = null;
// The open popup's mined badge, hidden until a card for the word is known to
// exist. Held here so a mine can raise it on a popup already on screen.
let minedBadge = null;
// Set by `render` while the open popup has more than one dictionary in it, so
// the wheel can page it without reaching into the arrows.
let stepSource = null;
// The rank at or under which an unknown word is called common. Fetched once;
// the same setting the reading view underlines by, so both agree.
let commonMaxRank = 0;
fetch("/api/settings")
  .then((r) => r.json())
  .then((s) => (commonMaxRank = s.reader_common_max_freq_rank || 0))
  .catch(() => {});

const stream = new EventSource("/api/lines/stream?backlog=1");

stream.onmessage = (e) => draw(JSON.parse(e.data));

stream.addEventListener("status", (e) => {
  const { capture } = JSON.parse(e.data);
  warnEl.textContent = capture === "live" ? "" : capture;
});

/** One line, one span per word the tokenizer found. */
function draw(incoming) {
  closePopup();
  line = incoming;
  const text = line.text;
  const frag = document.createDocumentFragment();
  let at = 0;

  // Offsets are UTF-16 code units, which is exactly what a JS string indexes
  // in, so they slice directly.
  for (const span of [...(line.tokens ?? [])].sort((a, b) => a.start - b.start)) {
    if (span.start < at) continue;
    if (span.start > at) frag.append(text.slice(at, span.start));

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
    word.textContent = text.slice(span.start, span.start + span.len);
    word.dataset.term = span.headword;
    word.dataset.reading = span.reading ?? "";
    word.dataset.status = span.status;
    frag.append(word);
    at = span.start + span.len;
  }
  frag.append(text.slice(at));

  lineEl.replaceChildren(frag);
  report();
}

lineEl.addEventListener("click", (e) => {
  const word = e.target.closest(".w");
  if (!word) return;
  e.stopPropagation();
  // A second click on the same word closes it, so one finger both opens and
  // dismisses without reaching anywhere else.
  if (word === openWord) return closePopup();
  show(word);
});

// Anywhere else on the surface dismisses. Not the popup itself, or scrolling a
// long Jitendex entry would close what is being read.
document.addEventListener("click", (e) => {
  if (!e.target.closest("#popup")) closePopup();
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
    if (!stepSource || !openWord || e.target.closest(".w") !== openWord) return;
    e.preventDefault();
    stepSource(Math.sign(e.deltaY));
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
const PLACE = "vn-overlay-offset";
let drag = null;
let offset = { x: 0, y: 0 };

try {
  offset = { ...offset, ...JSON.parse(localStorage.getItem(PLACE) ?? "{}") };
} catch {
  // Nothing stored, or stored by an older shape. Start where the CSS puts it.
}
apply();

/** Move to `x, y`, clamped so the box stays somewhere it can be grabbed back
 * from — the surface is the whole screen, and a strip pushed off it is gone. */
function moveTo(x, y) {
  const rect = lineEl.getBoundingClientRect();
  const left = rect.left - offset.x;
  const top = rect.top - offset.y;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  offset = {
    x: clamp(x, -left, Math.max(0, window.innerWidth - rect.width) - left),
    y: clamp(y, -top, Math.max(0, window.innerHeight - rect.height) - top),
  };
  apply();
}

function apply() {
  boxEl.style.setProperty("--dx", `${offset.x}px`);
  boxEl.style.setProperty("--dy", `${offset.y}px`);
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
  if (openWord) place(openWord);
});

for (const type of ["pointerup", "pointercancel"]) {
  lineEl.addEventListener(type, (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    lineEl.releasePointerCapture(e.pointerId);
    // The click that ends a drag is not a click on the overlay — without this
    // it reaches the document handler and closes the open popup.
    if (drag.moved) document.addEventListener("click", (c) => c.stopPropagation(), { capture: true, once: true });
    drag = null;
    localStorage.setItem(PLACE, JSON.stringify(offset));
  });
}

function closePopup() {
  popupEl.hidden = true;
  if (openWord) openWord.classList.remove("open");
  openWord = null;
  minedBadge = null;
  stepSource = null;
  report();
}

async function show(word) {
  const { term, reading } = word.dataset;
  if (openWord) openWord.classList.remove("open");
  openWord = word;
  word.classList.add("open");

  popupEl.hidden = false;
  popupEl.replaceChildren(el("div", "none", "…"));
  place(word);

  const query = new URLSearchParams({ term });
  if (reading) query.set("reading", reading);

  let data;
  try {
    const res = await fetch(`/api/reader/define?${query}`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    data = await res.json();
  } catch (err) {
    if (openWord === word) popupEl.replaceChildren(el("div", "none", `Lookup failed — ${err.message}`));
    return;
  }
  // A new line landed, or another word was clicked, while the fetch was out.
  if (openWord !== word) return;

  popupEl.replaceChildren(...render(data, word, reading));
  report();

  // Asked after the definition is on screen, not before it: Anki is a second
  // process and a slow or shut one must not hold up the answer to the question
  // actually being asked.
  try {
    const res = await fetch(`/api/reader/mined?term=${encodeURIComponent(term)}`);
    const { note_id } = await res.json();
    if (openWord === word) markMined(note_id);
  } catch {
    // Anki closed, or busy. The badge is an extra, never a report.
  }
}

/** Raise the open popup's "mined" badge, and point it at the card. */
function markMined(noteId) {
  if (!minedBadge || !noteId) return;
  minedBadge.hidden = false;
  minedBadge.onclick = () =>
    fetch("/api/reader/mined/browse", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ note_id: noteId }),
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
 * failed write is the tint coming back. */
async function judge(word, status) {
  const body = {
    judgements: [{ headword: word.dataset.term, reading: word.dataset.reading, status }],
  };
  const res = await fetch("/api/vocab/judge", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) return;
  word.dataset.status = status;
  word.classList.remove("new", "seen", "unknown");
  if (status !== "known") word.classList.add(status);
}

/** A card, built and added the way Yomitan's own add is.
 *
 * Silent: the chime is the only report a mine gets, here as everywhere, and it
 * plays once the capture and the CompactDef write have both come back. */
async function mine(word) {
  const res = await fetch("/api/reader/mine", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      term: word.dataset.term,
      reading: word.dataset.reading,
      surface: word.textContent,
      sentence: line ? line.text : "",
    }),
  });
  // The add answers with the new note's id, so a popup open on this word gets
  // its badge now rather than the next time it is opened.
  const { note_id } = await res.json().catch(() => ({}));
  if (openWord === word) markMined(note_id);
}

function render(data, word, reading) {
  const surface = word.textContent;
  const head = el("div", "head");
  head.append(el("span", "term", data.term));
  if (reading && reading !== data.term) head.append(el("span", "reading", reading));
  // NHK's downstep for this reading, the accent Yomitan would show.
  for (const p of data.pitch ?? []) {
    if (p.positions.length) head.append(el("span", "pitch", `[${p.positions.join("] [")}]`));
  }
  // The surface is worth showing only where it differs from the headword —
  // that difference is the conjugation the tokenizer saw through.
  if (surface !== data.term) head.append(el("span", "reading", `— ${surface}`));
  head.append(ranks(data));
  // Built hidden and kept, rather than added when the answer arrives: the
  // answer can arrive from two directions — Anki's duplicate check, or a mine
  // made while this popup is open — and both then have one thing to raise.
  stepSource = null;
  minedBadge = el("button", "mined", "mined");
  minedBadge.title = "Open the card in Anki";
  minedBadge.hidden = true;
  head.append(minedBadge);

  const out = [head];

  // One dictionary at a time. Sankoku says the same thing more briefly than
  // Jitendex does, and stacking both makes the popup a page to scroll rather
  // than an answer to read; the arrows are there for when the first one is the
  // wrong one.
  if (data.sources.length) {
    const body = el("div", "body");
    const label = el("span", "dict");
    const paging = el("div", "paging");
    const back = document.createElement("button");
    const next = document.createElement("button");
    back.textContent = "\u2039";
    next.textContent = "\u203a";

    let at = 0;
    const showSource = () => {
      const source = data.sources[at];
      label.textContent = source.dictionary;
      back.disabled = at === 0;
      next.disabled = at === data.sources.length - 1;
      const list = document.createElement("ol");
      list.className = "sense";
      for (const sense of source.senses) {
        for (const def of sense.definitions) {
          const item = document.createElement("li");
          // Jitendex ships HTML in its definitions; the master ships plain text.
          item.innerHTML = def;
          list.append(item);
        }
      }
      body.replaceChildren(list);
    };
    back.addEventListener("click", () => (at--, showSource()));
    next.addEventListener("click", () => (at++, showSource()));
    // Clamped rather than wrapped: the order is `define::OPENS_WITH`, so the
    // first entry is the one worth reading first and wrapping past the last
    // would land back on it as if it were a new answer.
    stepSource = (by) => {
      const to = Math.min(Math.max(at + by, 0), data.sources.length - 1);
      if (to === at) return;
      at = to;
      showSource();
    };

    const bar = el("div", "dictbar");
    bar.append(label);
    if (data.sources.length > 1) {
      paging.append(back, next);
      bar.append(paging);
    }
    showSource();
    out.push(bar, body);
  } else {
    out.push(el("div", "none", "Not in any dictionary"));
  }

  return out;
}

/** One pill per list, the name filled and the number not — Yomitan's shape.
 *
 * Two lists, two answers: jiten ranks the word in the fiction being read,
 * BCCWJ in newspaper and government prose. They disagree by an order of
 * magnitude on ordinary words, so the number is worth nothing without the name
 * attached to it. */
function ranks(data) {
  const out = el("div", "rank");
  for (const [name, rank] of [
    ["jiten", data.jiten],
    ["BCCWJ", data.bccwj],
  ]) {
    const pill = el("span", "freq");
    pill.append(el("span", "freq-name", name));
    pill.append(el("span", "freq-value", rank == null ? "—" : rank.toLocaleString("en")));
    out.append(pill);
  }
  return out;
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
  const rects = [lineEl.getBoundingClientRect()];
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
window.addEventListener("resize", report);

// Only under the overlay shell — in an ordinary browser there is no channel and
// the page is simply a page. qwebchannel.js is injected by the shell, so
// nothing is served for it here.
if (window.qt?.webChannelTransport) {
  new QWebChannel(window.qt.webChannelTransport, (channel) => {
    shell = channel.objects.shell;
    report();
  });
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
