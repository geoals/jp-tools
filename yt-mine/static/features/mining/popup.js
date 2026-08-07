// yt-mine's half of the dictionary popup.
//
// The popup itself is `web-shared/popup.js`, the same module the VN overlay
// loads — so a word looked up over a game and a word looked up in a transcript
// are the same popup, down to the per-dictionary styling. What is here is what
// is about *this* surface: an ordinary scrolling page, no side mouse buttons,
// and a ＋ that has a video and a timestamp to attach rather than a live
// screen to capture.
//
// It lives outside the Preact tree and is driven imperatively, because the
// shared module owns its own DOM. The row components only tell it which word
// was clicked.

import { createPopup } from '/shared/popup.js';
import { judged, exportedIds, exportResult } from './state.js';
import { judgeWord, exportSentences } from '../../api.js';

let popup = null;
let ctx = { videoId: null, jobId: null, sentenceId: null, text: '' };

function element() {
  let el = document.getElementById('word-popup');
  if (!el) {
    el = document.createElement('div');
    el.id = 'word-popup';
    el.className = 'jp-popup';
    el.hidden = true;
    document.body.append(el);
  }
  return el;
}

function instance() {
  if (popup) return popup;
  popup = createPopup({
    el: element(),
    // yt-mine's own routes: no lookup is recorded, and there is no live Anki
    // duplicate check on this path, so no mined badge.
    api: {
      define: (query) => `/api/define?${query}`,
      expand: (text) => `/api/expand?${new URLSearchParams({ text })}`,
    },
    scanText: (target) => ctx.text.slice(target.start ?? 0),
    judge: async (target, status) => {
      if (!(await judgeWord(target.key, target.reading, status))) return false;
      // The tokens came from the server with the status they had when the
      // sentence was fetched; this is what repaints them without a refetch.
      judged.value = new Map(judged.value).set(`${target.key} ${target.reading}`, status);
      return true;
    },
    mine: (target) => exportOne(target),
    place,
  });
  // Anywhere else dismisses. The popup stops its own clicks inside the shared
  // module, so this never has to ask where the click came from.
  document.addEventListener('click', () => close());
  document.addEventListener('keydown', (e) => e.key === 'Escape' && close());
  return popup;
}

/** Open on a word, or close if it is the word already open. */
export function toggle(anchor, target, next) {
  const p = instance();
  if (p.isOpen() && p.anchor() === anchor) return close();
  const previous = p.anchor();
  if (previous) previous.classList.remove('open');
  anchor.classList.add('open');
  ctx = next;
  p.show(anchor, target);
}

export function close() {
  if (!popup) return;
  const anchor = popup.anchor();
  if (anchor) anchor.classList.remove('open');
  popup.close();
}

/** The wheel over the open word pages its dictionaries — the hand is already
 * there, having just clicked it. Only over that word: anywhere else the wheel
 * still scrolls the page. */
export function wheel(e, anchor) {
  if (!popup || !popup.isOpen() || popup.anchor() !== anchor) return;
  e.preventDefault();
  popup.step(Math.sign(e.deltaY));
}

/** Anchored in *document* coordinates, so the popup stays on its word as the
 * page scrolls rather than hanging in the viewport where the word used to be.
 *
 * Below the word where there is room and above it otherwise, so a sentence near
 * the bottom of a long list still gets a readable popup. Clamped horizontally
 * so a word at either edge cannot push it off screen. */
function place(anchor) {
  const el = element();
  const rect = anchor.getBoundingClientRect();
  const width = el.offsetWidth;
  const height = el.offsetHeight;
  const left = rect.left + rect.width / 2 - width / 2;
  const room = window.innerHeight - rect.bottom;
  const top = room > height || rect.top < height ? rect.bottom + 6 : rect.top - height - 6;
  el.style.left = `${Math.max(8, Math.min(left, window.innerWidth - width - 8)) + window.scrollX}px`;
  el.style.top = `${top + window.scrollY}px`;
}

/** ＋ exports this sentence to Anki, now.
 *
 * Not a selection to commit later: a video is read a sentence at a time, and
 * the word being looked at is the word the card is about. `target` is what the
 * popup is open on, so a compound picked out of the scan (経年劣化) is what
 * gets mined rather than the token that was clicked. */
async function exportOne(target) {
  try {
    const result = await exportSentences(ctx.jobId, [
      { id: ctx.sentenceId, target_word: target.key, target_reading: target.reading },
    ]);
    exportedIds.value = new Set([...exportedIds.value, ...result.exported_ids]);
    exportResult.value = `${target.key} exported to Anki.`;
    close();
  } catch (err) {
    exportResult.value = `Error: ${err.message}`;
  }
  // No note id to answer with: the export goes through AnkiConnect in the
  // exporter, which reports a count rather than an id.
  return null;
}
