import { signal } from '@preact/signals';

// The open popup: { sentenceId, target, rect } or null.
//
// `target` is what every action in the popup acts on — `{ term, key, reading,
// surface, status }`. Not always the clicked word's own pair: picking a match
// out of the scan re-opens the popup on the same word under another term.
// `rect` is the clicked token's box, for placing the popup over it.
export const activePopup = signal(null);

// Ledger statuses written since the page loaded, keyed `headword\u0000reading`.
// The tokens come from the server with the status they had when the sentence
// was fetched, and judging one must repaint it without re-fetching the job.
export const judged = signal(new Map());

// Map<sentenceId, { key, reading }> — the card's word per sentence. The
// reading rides along because a word picked out of the popup's scan need not
// be a token, so the server cannot look it back up.
export const selectedWords = signal(new Map());

// Set<sentenceId> — sentences that have been exported
export const exportedIds = signal(new Set());

// { playing: bool, loading: bool, sentenceId: number|null }
export const audioState = signal({ playing: false, loading: false, sentenceId: null });

// Last export result message (string or null)
export const exportResult = signal(null);
