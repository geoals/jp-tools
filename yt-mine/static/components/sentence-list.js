import { html } from 'https://esm.sh/htm@3.1.1/preact/standalone';
import { useState } from 'https://esm.sh/preact@10.25.4/hooks';
import { SentenceRow } from './sentence-row.js';
import { WordPreview } from './word-preview.js';
import { ExportResult } from './export-result.js';
import { activePreview, selectedWords, exportedIds, exportResult } from '../state.js';
import { exportSentences } from '../api.js';

export function SentenceList({ sentences, videoId, jobId, isDone, isTranscribing }) {
  if (!sentences || sentences.length === 0) return null;

  const [exporting, setExporting] = useState(false);
  const preview = activePreview.value;
  const selected = selectedWords.value;

  async function handleExport() {
    const entries = [];
    for (const [sentenceId, targetWord] of selected) {
      entries.push({ id: sentenceId, target_word: targetWord });
    }
    // Also include sentences without a selected word? No — export only selected.
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
      // Close preview if it was on an exported sentence
      if (preview && result.exported_ids.includes(preview.sentenceId)) {
        activePreview.value = null;
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
        ${preview && preview.sentenceId === s.id && html`
          <li class="preview-container" key="preview-${s.id}">
            <${WordPreview}
              videoId=${videoId}
              sentenceId=${s.id}
              word=${preview.word}
            />
          </li>
        `}
      `)}
    </ul>
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
