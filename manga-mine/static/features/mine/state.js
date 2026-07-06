import { signal } from '@preact/signals';

/** Number of fire-and-forget exports still running in the background. */
export const exportsPending = signal(0);

/** Last background export failure, e.g. "強: Export to Anki failed." */
export const exportError = signal(null);

/** Bumped when a background export completes so the queue view refreshes. */
export const queueVersion = signal(0);
