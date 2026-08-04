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

const params = new URLSearchParams(location.search);
const root = document.documentElement.style;
root.setProperty("--backdrop", `rgba(0, 0, 0, ${params.get("bg") ?? "0.55"})`);
root.setProperty("--strip", `${params.get("h") ?? "300"}px`);

const lineEl = document.getElementById("line");
const warnEl = document.getElementById("warn");
const popupEl = document.getElementById("popup");

let openWord = null;
let line = null;
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

function closePopup() {
  popupEl.hidden = true;
  if (openWord) openWord.classList.remove("open");
  openWord = null;
}

async function show(word) {
  const { term, reading } = word.dataset;
  if (openWord) openWord.classList.remove("open");
  openWord = word;
  word.classList.add("open");

  // Anchored to the word rather than centred, and clamped so a word at either
  // end of the line cannot push the popup off screen.
  const rect = word.getBoundingClientRect();
  popupEl.hidden = false;
  popupEl.replaceChildren(el("div", "none", "…"));
  const width = popupEl.offsetWidth;
  popupEl.style.left = `${Math.max(
    12,
    Math.min(rect.left - width / 3, window.innerWidth - width - 12),
  )}px`;

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
}

/** Known / unknown, written to the same ledger the reading view writes to. */
async function judge(word, status, button) {
  const body = {
    judgements: [{ headword: word.dataset.term, reading: word.dataset.reading, status }],
  };
  const res = await fetch("/api/vocab/judge", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  button.textContent = res.ok ? `${status} \u2713` : "failed";
  button.classList.toggle("done", res.ok);
  if (!res.ok) return;
  // Repaint the word under the popup so the tint agrees with what was just
  // asserted, without waiting for the next line.
  word.dataset.status = status;
  word.classList.remove("new", "seen", "unknown");
  if (status !== "known") word.classList.add(status);
}

/** A card, built and added the way Yomitan's own add is. */
async function mine(word, button) {
  button.textContent = "mining\u2026";
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
  const ok = res.ok && (await res.json().catch(() => ({}))).ok;
  button.textContent = ok ? "mined \u2713" : "mine failed";
  button.classList.toggle("done", !!ok);
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
  head.append(el("span", "rank", ranks(data)));

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

  const actions = el("div", "actions");
  for (const [label, run] of [
    ["known", (b) => judge(word, "known", b)],
    ["unknown", (b) => judge(word, "unknown", b)],
    ["mine", (b) => mine(word, b)],
  ]) {
    const button = document.createElement("button");
    button.textContent = label;
    button.addEventListener("click", () => run(button));
    actions.append(button);
  }
  out.push(actions);
  return out;
}

function ranks(data) {
  const n = (v) => (v == null ? "—" : v.toLocaleString("en"));
  return `jiten ${n(data.jiten)} · BCCWJ ${n(data.bccwj)}`;
}

/** Every rectangle on this page that should take a click, in window pixels.
 *
 * The shell polls this and hands it to `wl_surface.set_input_region`, so the
 * overlay is clickable exactly where a word is and the VN gets everything else
 * — clicking on to the next line never touches the overlay at all. CSS pixels
 * and window pixels are the same thing here: the view fills the surface.
 */
window.__hitRects = () => {
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
  return rects.flatMap((r) => [r.left - 4, r.top - 4, r.width + 8, r.height + 8]);
};

function el(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
