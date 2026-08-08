// What the ledger says about a token, and what a line is worth stopping on.
//
// The tokens arrive with the status they had when the sentence was fetched, so
// anything judged since the page loaded is read out of `judged` on top.

import { judged } from './state.js';

export function key(tok) {
  return `${tok.base_form} ${tok.reading}`;
}

export function statusOf(tok, marks = judged.value) {
  return marks.get(key(tok)) ?? tok.status;
}

/** The content words in a line that are not claimed as known.
 *
 * `new` is not `unknown` in the ledger — one was never judged, the other was
 * judged and refused — but both are words this page exists to do something
 * about, so the filters and the mark-known button treat them alike. */
export function notKnown(sentence, marks) {
  return sentence.tokens.filter((t) => {
    if (!t.is_content_word) return false;
    const status = statusOf(t, marks);
    return status !== 'known' && status !== 'blacklisted';
  });
}

export const FILTERS = [
  ['all', 'All'],
  ['unknown', 'Unknown'],
  ['i+1', 'i+1'],
];

export function matchesFilter(sentence, filter, marks) {
  if (filter === 'all') return true;
  const count = notKnown(sentence, marks).length;
  return filter === 'i+1' ? count === 1 : count > 0;
}
