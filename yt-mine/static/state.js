import { signal } from 'https://esm.sh/@preact/signals@1.3.1?deps=preact@10.25.4';

// { sentenceId, word } or null
export const activePreview = signal(null);

// Map<sentenceId, baseForm> — selected target words per sentence
export const selectedWords = signal(new Map());

// Set<sentenceId> — sentences that have been exported
export const exportedIds = signal(new Set());

// { playing: bool, loading: bool, sentenceId: number|null }
export const audioState = signal({ playing: false, loading: false, sentenceId: null });

// Last export result message (string or null)
export const exportResult = signal(null);
