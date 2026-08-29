// How much of a transcript is already understood.
//
// Every figure counts a word as understood only when the ledger asserts it —
// `known`, or mined. `new` and `seen` mean never judged, so the figures are a
// floor: the true number is higher by however much of the unjudged mass is
// already understood. `left` is the size of that mass, and is why it is
// reported beside the percentages rather than derived from them.

import { key, statusOf } from './ledger.js';

function isKnown(status) {
  return status === 'known' || status === 'blacklisted';
}

/** Comprehensibility of `sentences`, as shares of what is asserted known.
 *
 * Lines with no content word at all — an interjection, a stray 「はい」 — are
 * left out entirely rather than counted as fully understood, which would
 * inflate the line figure with lines that assert nothing. */
export function comprehension(sentences, marks) {
  const types = new Map();
  let words = 0;
  let wordsKnown = 0;
  let lines = 0;
  let linesKnown = 0;

  for (const s of sentences) {
    const content = s.tokens.filter((t) => t.is_content_word);
    if (!content.length) continue;
    lines++;
    let gaps = 0;
    for (const t of content) {
      const known = isKnown(statusOf(t, marks));
      words++;
      if (known) wordsKnown++;
      else gaps++;
      types.set(key(t), known);
    }
    if (gaps === 0) linesKnown++;
  }

  const typesKnown = [...types.values()].filter(Boolean).length;

  return {
    words: share(wordsKnown, words),
    types: share(typesKnown, types.size),
    lines: share(linesKnown, lines),
    // What is left to do, not what is done: the count the percentages hide. The
    // same percentage is a handful of words in a short video and hundreds in a
    // long one.
    left: types.size - typesKnown,
  };
}

function share(known, total) {
  return { total, known, pct: total ? known / total : 0 };
}
