import { html } from 'htm/preact';
import { useState } from 'preact/hooks';
import { SentenceRow } from './sentence-row.js';
import { WordPopup } from './word-popup.js';
import { ExportResult } from './export-result.js';
import { activePopup, selectedWords, exportedIds, exportResult } from './state.js';
import { exportSentences } from '../../api.js';

export function SentenceList({ sentences, videoId, jobId, isDone, isTranscribing }) {
  if (!sentences || sentences.length === 0) return null;

  const [exporting, setExporting] = useState(false);
  const popup = activePopup.value;
  const selected = selectedWords.value;

  // The rows are built fresh every render, and must be. This used to cache the
  // VNode per sentence and hand the same reference back, which makes Preact
  // skip the whole subtree — including when a signal the row reads has
  // changed. A word judged in the popup kept its old tint, and the token the
  // popup was open on stayed outlined after it closed.

  async function handleExport() {
    const entries = [];
    for (const [sentenceId, word] of selected) {
      entries.push({ id: sentenceId, target_word: word.key, target_reading: word.reading });
    }
    if (entries.length === 0) {
      exportResult.value = 'Error: No words selected. Click a word in a sentence first.';
      return;
    }

    setExporting(true);
    exportResult.value = null;
    try {
      const result = await exportSentences(jobId, entries);
      exportResult.value = `${result.count} sentence(s) exported to Anki.`;
      // Mark exported
      const next = new Set(exportedIds.value);
      for (const id of result.exported_ids) next.add(id);
      exportedIds.value = next;
      // Clear selections for exported sentences
      const nextSelected = new Map(selected);
      for (const id of result.exported_ids) nextSelected.delete(id);
      selectedWords.value = nextSelected;
      // Close the popup if it was on an exported sentence
      if (popup && result.exported_ids.includes(popup.sentenceId)) {
        activePopup.value = null;
      }
    } catch (err) {
      exportResult.value = `Error: ${err.message}`;
    } finally {
      setExporting(false);
    }
  }

  const hasSelections = selected.size > 0;

  return html`
    <ul class="sentence-list ${isTranscribing ? 'transcribing' : ''}">
      ${sentences.map((s) => html`
        <${SentenceRow}
          key=${s.id}
          sentence=${s}
          videoId=${videoId}
          isTranscribing=${isTranscribing}
        />
      `)}
    </ul>
    ${popup && html`
      <${WordPopup}
        key=${`${popup.sentenceId}-${popup.target.term}-${popup.target.reading}`}
        videoId=${videoId}
        sentenceId=${popup.sentenceId}
        sentence=${(sentences.find((s) => s.id === popup.sentenceId) || {}).text || ''}
        target=${popup.target}
        rect=${popup.rect}
      />
    `}
    <${ExportResult} />
    <button
      type="button"
      onClick=${handleExport}
      disabled=${exporting || !hasSelections}
    >
      ${exporting && html`<span class="spinner"></span>`}
      <span>Export to Anki</span>
    </button>
  `;
}
