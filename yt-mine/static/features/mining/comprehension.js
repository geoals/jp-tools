// How much of a transcript is already understood.
//
// Every figure is a **range**, and the range is the point. The ledger's `new`
// and `seen` mean *never judged*, not *not known*, and a fresh transcript is
// almost entirely `seen` — so a single number would report a video read
// comfortably as 5% comprehensible. The floor counts only what is asserted
// known; the ceiling grants every unjudged word. The truth is between them, and
// the gap closes as the video is worked through.

import { key, statusOf } from './ledger.js';

const KNOWN = 'known';
const UNJUDGED = 'unjudged';
const REFUSED = 'refused';

function classify(status) {
  if (status === 'known' || status === 'blacklisted') return KNOWN;
  if (status === 'unknown') return REFUSED;
  return UNJUDGED;
}

/** Comprehensibility of `sentences`, as ranges of counts.
 *
 * Lines with no content word at all — an interjection, a stray 「はい」 — are
 * left out entirely rather than counted as fully understood, which would
 * inflate the line figure with lines that assert nothing. */
export function comprehension(sentences, marks) {
  const types = new Map();
  let words = 0;
  let wordsKnown = 0;
  let wordsUnjudged = 0;
  let lines = 0;
  let linesKnown = 0;
  let linesNoRefused = 0;

  for (const s of sentences) {
    const content = s.tokens.filter((t) => t.is_content_word);
    if (!content.length) continue;
    lines++;
    let gaps = 0;
    let refused = 0;
    for (const t of content) {
      const cls = classify(statusOf(t, marks));
      words++;
      if (cls === KNOWN) wordsKnown++;
      else {
        gaps++;
        if (cls === UNJUDGED) wordsUnjudged++;
        else refused++;
      }
      types.set(key(t), cls);
    }
    if (gaps === 0) linesKnown++;
    if (refused === 0) linesNoRefused++;
  }

  const typeValues = [...types.values()];
  const typesKnown = typeValues.filter((c) => c === KNOWN).length;
  const typesUnjudged = typeValues.filter((c) => c === UNJUDGED).length;

  return {
    words: range(wordsKnown, wordsKnown + wordsUnjudged, words),
    types: range(typesKnown, typesKnown + typesUnjudged, types.size),
    lines: range(linesKnown, linesNoRefused, lines),
    // What is left to do, not what is done: the count the percentages hide.
    // 6% unknown is forty words in one video and four hundred in another.
    typesLeft: types.size - typesKnown,
    typesRefused: typeValues.length - typesKnown - typesUnjudged,
  };
}

function range(floor, ceiling, total) {
  return {
    total,
    floor: total ? floor / total : 0,
    ceiling: total ? ceiling / total : 0,
    exact: floor === ceiling,
  };
}
