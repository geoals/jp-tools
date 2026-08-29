import { html } from 'htm/preact';
import { SentenceRow } from './sentence-row.js';
import { ExportResult } from './export-result.js';

// No bulk selection and no export button: ＋ in the popup exports that sentence
// immediately. A video is read a sentence at a time, and the word being looked
// at is the word the card is about — there was never a batch to assemble.
export function SentenceList({ sentences, videoId, jobId, isTranscribing }) {
  if (!sentences || sentences.length === 0) return null;

  // The rows are built fresh every render, and must be. Caching a VNode per
  // sentence and handing the same reference back makes Preact skip the whole
  // subtree — including when a signal the row reads has changed, which leaves a
  // judged word on its old tint and the popup's token outlined after it closes.
  return html`
    <ul class="sentence-list ${isTranscribing ? 'transcribing' : ''}">
      ${sentences.map((s) => html`
        <${SentenceRow} key=${s.id} sentence=${s} videoId=${videoId} jobId=${jobId} />
      `)}
    </ul>
    <${ExportResult} />
  `;
}
