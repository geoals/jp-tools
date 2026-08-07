import { signal } from '@preact/signals';

// Ledger statuses written since the page loaded, keyed `headword\u0000reading`.
// The tokens come from the server with the status they had when the sentence
// was fetched, and judging one must repaint it without re-fetching the job.
export const judged = signal(new Map());

// Set<sentenceId> — sentences that have been exported
export const exportedIds = signal(new Set());

// { playing: bool, loading: bool, sentenceId: number|null }
export const audioState = signal({ playing: false, loading: false, sentenceId: null });

// Last export result message (string or null)
export const exportResult = signal(null);
