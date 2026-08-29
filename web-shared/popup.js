// The dictionary popup, shared by the VN overlay and yt-mine.
//
// One implementation because it is one thing: the word, its readings and
// pitch, the two frequency lists, a chip per other reading the dictionaries
// offer for this position, and one dictionary at a time with arrows to page
// them. It renders `jp_core::define`'s answer, which both hosts serve.
//
// What is *not* here is anything about the surface it sits on. The overlay
// draws over a game in a layer-shell strip, mines a card the moment ＋ is hit,
// and has side mouse buttons that judge without opening anything; yt-mine is
// an ordinary scrolling page with none of that. So the host supplies the
// placement, the two writes (`judge`, `mine`), and the text the expansion scan
// reads — and gets back an object it opens, closes and pages.
//
// Vanilla DOM on purpose. One host is a plain page and the other is Preact,
// and the way to share between those is to belong to neither.

/** @param {Object} opts
 *  @param {HTMLElement} opts.el   the popup element, `class="jp-popup"`, hidden
 *  @param {(target) => string} opts.scanText  the raw line from the word's
 *     first character on — what the expansion scan reads
 *  @param {(target, status) => Promise<boolean>} opts.judge
 *  @param {(target) => Promise<number|null|undefined>} [opts.mine]  the new note
 *      id. Null draws no ＋ — a host with nowhere to add a card.
 *  @param {(anchor) => void} opts.place   host positioning, called on open
 *  @param {Object} [opts.api]      url builders; see API_DEFAULTS
 *  @param {(data, target) => void} [opts.onOpen]
 *  @param {(target, status) => void} [opts.onJudged]
 *  @param {() => void} [opts.onLayout]  after anything changes the popup's size
 */
export function createPopup(opts) {
  const api = { ...API_DEFAULTS, ...(opts.api ?? {}) };
  const { el: popupEl, scanText, judge, mine, place } = opts;
  // A host can lose the ability to add a card while the page is open — Anki
  // quits — so this is a switch rather than only the presence of `mine`.
  let mining = true;
  const onOpen = opts.onOpen ?? (() => {});
  const onJudged = opts.onJudged ?? (() => {});
  const onLayout = opts.onLayout ?? (() => {});

  // The word the popup is open on, and what it is *about* — not always the
  // same pair: picking a match re-opens it on the same word under another
  // term. Every action reads `target`. `key` is what the ledger is keyed on and
  // `term` what the dictionary calls it; they differ only for a picked match,
  // whose spelling comes from outside the tokenizer.
  let anchor = null;
  let target = null;
  // Set while the open popup holds more than one dictionary, so the host's
  // wheel handler can page it without reaching into the arrows.
  let stepSource = null;
  // Hidden until a card for the word is known to exist. Held so a mine can
  // raise it on a popup already on screen.
  let minedBadge = null;
  // ＋. Held for the same reason as the badge, and because the two are one
  // state: the badge appearing is what removes the button.
  let addButton = null;
  // The speaker button, and the clip it plays. Built hidden and filled in when
  // the source list lands, which is the same shape as the mined badge and for
  // the same reason:
  // most words have audio, some have none, and the answer arrives after the
  // definition is already on screen. The clip is held so closing the popup can
  // stop it — the word is gone, and so is the reason to hear it.
  let audioButton = null;
  let clip = null;

  function close() {
    popupEl.hidden = true;
    anchor = null;
    target = null;
    minedBadge = null;
    addButton = null;
    audioButton = null;
    clip?.pause();
    clip = null;
    stepSource = null;
    onLayout();
  }

  async function show(nextAnchor, nextTarget) {
    anchor = nextAnchor;
    target = nextTarget;
    const mine_ = target;

    popupEl.hidden = false;
    popupEl.replaceChildren(el("div", "none", "…"));
    place(anchor);

    const query = new URLSearchParams({ term: target.term });
    if (target.reading) query.set("reading", target.reading);

    // Started with the definition, not behind a button: the row only appears
    // when there is another match to offer, and that answer has to be in
    // before the popup can know whether to draw it. An empty list is the
    // common case and draws nothing.
    // A cross-reference carries its own scan text: the word it names is not
    // in the line, so slicing the line from `start` would offer the chips of
    // whatever was clicked first.
    const matches = fetch(api.expand(target.scan ?? scanText(target)))
      .then((r) => r.json())
      .catch(() => []);

    let data;
    try {
      const res = await fetch(api.define(query));
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      data = await res.json();
    } catch (err) {
      if (target === mine_) {
        popupEl.replaceChildren(
          el("div", "none", `Lookup failed — ${err.message}`),
        );
      }
      return;
    }
    // Another word — or another expansion of this one — was asked about while
    // the fetch was out.
    if (target !== mine_) return;

    onOpen(data, target);
    popupEl.replaceChildren(...render(data, matches));
    // Placed again now there is something to measure. The first call had the
    // "…" placeholder in it, so every host asking "does it fit above the word?"
    // was answering for a box one line tall.
    place(anchor);
    onLayout();

    loadAudio(mine_);

    if (!api.mined) return;
    // Asked after the definition is on screen, not before it: Anki is a second
    // process and a slow or shut one must not hold up the answer to the
    // question actually being asked.
    try {
      // The key, not the dictionary's spelling: that is what a mine writes
      // into the card's vocab field, so it is what the duplicate check asks.
      const res = await fetch(api.mined(target.key));
      const { note_id } = await res.json();
      if (target === mine_) markMined(note_id);
    } catch {
      // Anki closed, or busy. The badge is an extra, never a report.
    }
  }

  /** Raise the open popup's "mined" badge, and point it at the card.
   *
   * ＋ goes with it. Mining a word that is already a card is the duplicate
   * Anki would refuse anyway, so the badge and the button are one state: what
   * the head says about the card is either "make one" or "here it is". */
  function markMined(noteId) {
    if (!minedBadge || !noteId) return;
    minedBadge.hidden = false;
    minedBadge.classList.add("to-card");
    minedBadge.title = "Open this card in Anki";
    // Closes with it: the card is the answer, and it opens in Anki — leaving
    // the popup standing over a word the reader has finished with.
    minedBadge.onclick = () => {
      api.browse(noteId);
      close();
    };
    if (addButton) addButton.hidden = true;
    onLayout();
  }

  /** Take the speaker button away again: there is a source but no sound. */
  function hideAudio() {
    clip = null;
    if (!audioButton) return;
    audioButton.hidden = true;
    onLayout();
  }

  /** Find this word's audio and arm the speaker button with the first clip.
   *
   * The audio server ranks its own sources — NHK before 新明解 before Forvo —
   * so the first is the one to play, and the button names it rather than
   * offering a list. A word with no recording is the ordinary case and leaves
   * the button hidden.
   *
   * The clip is preloaded rather than fetched on the click: it is tens of
   * kilobytes off the same machine, and a pronunciation that starts a moment
   * after the press reads as a press that missed. */
  async function loadAudio(on) {
    if (!api.audio) return;
    let sources;
    try {
      const res = await fetch(api.audio(on.term, on.reading ?? ""));
      ({ sources } = await res.json());
    } catch {
      return; // No audio server. It is the one thing here that is optional.
    }
    if (target !== on || !audioButton || !sources?.length) return;
    const [first] = sources;
    const playing = new Audio(api.audioClip(first.clip));
    clip = playing;
    clip.preload = "auto";
    // A source list is not a playable clip: the server can name a file it can
    // no longer serve, and a button that answers a press with nothing is worse
    // than no button. Only while this is still the clip the button plays — a
    // later word's failure is not this one's.
    playing.addEventListener("error", () => {
      if (clip === playing) hideAudio();
    });
    audioButton.title = `Play — ${first.name}`;
    audioButton.hidden = false;
    onLayout();
  }

  function render(data, matches) {
    const { reading, surface } = target;
    const head = el("div", "head");
    head.append(el("span", "term", data.term));
    if (reading && reading !== data.term)
      head.append(el("span", "reading", reading));
    // NHK's downstep for this reading, the accent Yomitan would show.
    for (const p of data.pitch ?? []) {
      if (p.positions.length)
        head.append(el("span", "pitch", `[${p.positions.join("] [")}]`));
    }
    // The surface is worth showing only where it differs from the headword —
    // that difference is the conjugation the tokenizer saw through.
    if (surface !== data.term)
      head.append(el("span", "reading", `— ${surface}`));
    const rankRow = ranks(data);
    if (rankRow) head.append(rankRow);
    // Built hidden and kept, rather than added when the answer arrives: the
    // answer can arrive from two directions — Anki's duplicate check, or a mine
    // made while this popup is open — and both then have one thing to raise.
    stepSource = null;
    // An anchor, not a button: it is a way to the card, and it is the only
    // thing in the head that leaves the page. It carries no `href` — Anki has
    // no URL — so `markMined` is what makes it a link, and until then it is
    // inert and styled as inert.
    minedBadge = el("a", "mined", "mined");
    minedBadge.hidden = true;
    head.append(minedBadge);
    head.append(actions());

    const out = [head, expansions(matches)];

    // One dictionary at a time. Sankoku says the same thing more briefly than
    // Jitendex does, and stacking both makes the popup a page to scroll rather
    // than an answer to read; the arrows are there for when the first one is
    // the wrong one.
    if (data.sources.length) {
      const body = el("div", "body");
      const label = el("span", "dict");
      const paging = el("div", "paging");
      const back = document.createElement("button");
      const next = document.createElement("button");
      back.textContent = "‹";
      next.textContent = "›";

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
        // Each dictionary's markup means something different by the same
        // attribute, so the stylesheet keys its rules on this.
        body.dataset.dict = source.slug;
        body.replaceChildren(list);
        // On the body rather than each link: paging replaces the list, and the
        // links are inside markup the dictionary wrote.
        body.onclick = followLink;
        // Paging swaps in a definition of a different height, and the host
        // placed this one against the last one's. `isConnected` skips the
        // build: the first call runs before the popup is in the document.
        if (body.isConnected) place(anchor);
      };
      back.addEventListener("click", () => (at--, showSource()));
      next.addEventListener("click", () => (at++, showSource()));
      // Clamped rather than wrapped: the order is set by `jp_core::define`,
      // so the first entry is the one worth reading first and wrapping past the
      // last would land back on it as if it were a new answer.
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

  /** Follow a dictionary's own link.
   *
   * Yomitan writes cross-references as ordinary links — `?query=<term>`, with
   * `primary_reading` where the spelling has several — and Sankoku and
   * Jitendex are both full of them. Neither surface may navigate: one is a
   * layer over a game and the other a page with a video on it. So a
   * cross-reference re-opens the popup on that word, and a link out goes to a
   * tab of its own.
   */
  async function followLink(e) {
    const link = e.target.closest("a[href]");
    if (!link) return;
    const href = link.getAttribute("href");
    e.preventDefault();
    if (!href.startsWith("?")) {
      window.open(href, "_blank", "noopener");
      return;
    }
    const params = new URLSearchParams(href.slice(1));
    const term = params.get("query");
    if (!term) return;
    show(anchor, await linkTarget(term, params.get("primary_reading") ?? ""));
  }

  /** What the ledger calls the word a link names.
   *
   * A dictionary headword is text from outside the tokenizer, so judging or
   * mining it on the raw string would write a row that reads as never
   * encountered. The expansion scan already answers this — key and status for
   * a spelling — so the link asks it about its own term. With no answer the
   * word keys on itself, which is what a lookup with no ledger row is anyway.
   */
  async function linkTarget(term, reading) {
    const from = { surface: term, scan: term, start: target?.start };
    try {
      const res = await fetch(api.expand(term));
      const found = await res.json();
      const hit =
        found.find((e) => e.term === term && e.reading === reading) ??
        found.find((e) => e.term === term);
      if (hit)
        return {
          ...from,
          term: hit.term,
          key: hit.key,
          reading: hit.reading,
          status: hit.status,
        };
    } catch {
      // The scan is an extra here, never the answer.
    }
    return { ...from, term, key: term, reading, status: "new" };
  }

  /** "That is not the word here" — the escape hatch Yomitan gave for free.
   *
   * Two ways the tokenizer can be wrong about a position, and one answer to
   * both. It splits 経年劣化 into 経年 and 劣化, both real words, so nothing
   * downstream can join them. And it picks one reading for a spelling that has
   * several — 素振り as すぶり where the line means そぶり — which is a different
   * word with a different ledger row.
   *
   * So the scan offers every `(term, reading)` a dictionary lists for a prefix
   * of the line from this word on. Picking one re-opens the popup on it, and
   * ✓ ✗ ＋ then act on that term. The row draws nothing at all when the only
   * match is the one already showing, which is most words.
   */
  function expansions(matches) {
    const row = el("div", "expansions");
    const mine_ = target;
    matches.then((found) => {
      // The popup moved on while the scan was out.
      if (target !== mine_ || !Array.isArray(found)) return;
      // Minus the one already showing, which is a match of itself.
      const others = found.filter(
        (e) => !(e.term === target.term && e.reading === target.reading),
      );
      row.replaceChildren(
        ...others.map((e) => {
          const label =
            e.reading && e.reading !== e.term
              ? `${e.term}・${e.reading}`
              : e.term;
          const chip = el("button", "", label);
          chip.title = e.dictionaries.join(", ");
          chip.addEventListener("click", () =>
            show(anchor, {
              term: e.term,
              key: e.key,
              reading: e.reading,
              surface: e.term,
              status: e.status,
              start: target.start,
            }),
          );
          return chip;
        }),
      );
      // The popup just changed height.
      onLayout();
    });
    return row;
  }

  /** Known, unknown, mine — as buttons in the popup head.
   *
   * One character each, on the frequency pills' own metrics: the popup is over
   * a game on one of its two hosts, and everything in it costs line.
   */
  function actions() {
    const out = el("div", "acts");
    const on = target;

    // First in the row, and the only one of these that writes nothing. Hidden
    // until `loadAudio` finds a clip, so a word with no recording shows no
    // button rather than one that does nothing.
    audioButton = el("button", "audio");
    // Inline SVG, not an emoji: the emoji renders as a colour glyph the font
    // picks, and this row is monochrome text.
    audioButton.innerHTML =
      '<svg viewBox="0 0 16 16" aria-hidden="true">' +
      '<path d="M8.5 2.2 4.8 5.3H2.3v5.4h2.5l3.7 3.1z"/>' +
      '<path d="M11 5.6a3.4 3.4 0 0 1 0 4.8M13.2 3.4a6.5 6.5 0 0 1 0 9.2" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>' +
      "</svg>";
    audioButton.hidden = true;
    audioButton.addEventListener("click", () => {
      if (!clip) return;
      // From the top every time: the second press means "again", and a clip
      // played to its end would otherwise sit at the end and play nothing.
      clip.currentTime = 0;
      clip.play().catch(hideAudio);
    });
    out.append(audioButton);

    const mark = async (status) => {
      if (!(await judge(on, status))) return;
      on.status = status;
      for (const b of out.children)
        b.classList.toggle("on", b.dataset.status === status);
      onJudged(on, status);
    };
    // The status of the term the popup is about, which for a picked match is
    // not the clicked word's: しびれを切らす can be known while 痺れ is not.
    for (const [label, status, title] of [
      ["✓", "known", "Known"],
      ["✗", "unknown", "Unknown"],
    ]) {
      const b = el("button", on.status === status ? "on" : "", label);
      b.dataset.status = status;
      b.title = title;
      b.addEventListener("click", () => mark(status));
      out.append(b);
    }
    // A host with nowhere to send a card passes no `mine`, and the button is
    // simply not there — the same shape as `mined` and `audio`.
    if (!mine || !mining) return out;
    const add = el("button", "", "＋");
    add.title = "Mine";
    // A mine cuts an audio clip and a screenshot before Anki is asked, which
    // is seconds — long enough that a button doing nothing reads as a button
    // that missed the click. The spinner is what says it was heard.
    add.addEventListener("click", async () => {
      if (add.disabled) return;
      add.disabled = true;
      add.replaceChildren(el("span", "spin"));
      let noteId;
      try {
        noteId = await mine(on);
        markMined(noteId);
      } finally {
        add.disabled = false;
        // A refusal leaves the button where it was, which reads exactly like a
        // click that never landed. The host says *why* — it is the only side
        // that has Anki's answer — and this says *that*, so the two are not
        // both silent.
        add.textContent = noteId ? "＋" : "✕";
        add.classList.toggle("failed", !noteId);
        if (!noteId) setTimeout(() => {
          add.textContent = "＋";
          add.classList.remove("failed");
        }, 4000);
      }
    });
    addButton = add;
    out.append(add);
    return out;
  }

  // Not the popup itself, or scrolling a long Jitendex entry would close what
  // is being read.
  //
  // The popup stops its own clicks rather than the host's handler testing where
  // they came from. Testing does not work: `closest()` answers about where the
  // target is *now*, and picking another match re-renders the popup from inside
  // the click, which detaches the chip mid-dispatch — the detached chip then
  // reads as a click outside, so every pick closed the popup it had just
  // opened.
  popupEl.addEventListener("click", (e) => e.stopPropagation());

  return {
    show,
    close,
    markMined,
    /** Draw the ＋ or not — whether there is anywhere to add a card right now. */
    setMining: (on) => {
      mining = !!on;
    },
    /** Page the dictionaries. No-op unless the open popup has more than one. */
    step: (by) => stepSource && stepSource(by),
    isOpen: () => !popupEl.hidden,
    anchor: () => anchor,
    target: () => target,
  };
}

/** One pill per frequency dictionary, the name filled and the number not —
 * Yomitan's shape. Null where none is installed, and the head draws no row.
 *
 * The name is the dictionary's own, because two lists give two answers: a
 * fiction list and a newspaper corpus disagree by an order of magnitude on
 * ordinary words, so the number is worth nothing without the name attached. */
function ranks(data) {
  const lists = data.frequencies || [];
  if (!lists.length) return null;
  const out = el("div", "rank");
  for (const { dictionary, rank } of lists) {
    const pill = el("span", "freq");
    pill.append(el("span", "freq-name", dictionary));
    pill.append(
      el("span", "freq-value", rank == null ? "—" : rank.toLocaleString("en")),
    );
    out.append(pill);
  }
  return out;
}

const API_DEFAULTS = {
  define: (query) => `/api/define?${query}`,
  expand: (text) => `/api/expand?${new URLSearchParams({ text })}`,
  /** Null disables the mined badge — a host with no duplicate check to ask. */
  mined: null,
  browse: () => {},
  /** Null disables the speaker button — a host with no audio server in front of it. The pair
   *  goes together: the list names clips, `audioClip` is where they play from. */
  audio: null,
  audioClip: (path) => `/api/audio/clip?${new URLSearchParams({ path })}`,
};

export function el(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
