import { signal } from '@preact/signals';

// Ledger statuses written since the page loaded, keyed `headword\u0000reading`.
// The tokens come from the server with the status they had when the sentence
// was fetched, and judging one must repaint it without re-fetching the job.
export const judged = signal(new Map());

export const exportedIds = signal(new Set());

export const audioState = signal({ playing: false, loading: false, sentenceId: null });

export const exportResult = signal(null);
