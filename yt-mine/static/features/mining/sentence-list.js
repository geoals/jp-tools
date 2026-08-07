import { html } from 'htm/preact';
import { SentenceRow } from './sentence-row.js';
import { ExportResult } from './export-result.js';

// No bulk selection and no export button: ＋ in the popup exports that sentence
// immediately. A video is read a sentence at a time, and the word being looked
// at is the word the card is about — there was never a batch to assemble.
export function SentenceList({
  sentences,
  videoId,
  jobId,
  isTranscribing,
  refining,
  onSharpen,
}) {
  if (!sentences || sentences.length === 0) return null;

  // The rows are built fresh every render, and must be. This used to cache the
  // VNode per sentence and hand the same reference back, which makes Preact
  // skip the whole subtree — including when a signal the row reads has
  // changed. A word judged in the popup kept its old tint, and the token the
  // popup was open on stayed outlined after it closed.
  return html`
    <ul class="sentence-list ${isTranscribing ? 'transcribing' : ''}">
      ${sentences.map((s) => html`
        <${SentenceRow}
          key=${s.id}
          sentence=${s}
          videoId=${videoId}
          jobId=${jobId}
          isRefining=${refining != null && Math.abs(s.start_seconds - refining) <= 25}
          onSharpen=${onSharpen}
        />
      `)}
    </ul>
    <${ExportResult} />
  `;
}
