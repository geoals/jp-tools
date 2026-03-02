import { html } from 'htm/preact';
import { activePreview, selectedWords, exportedIds, audioState } from './state.js';

export function SentenceRow({ sentence, videoId, isTranscribing }) {
  const preview = activePreview.value;
  const selected = selectedWords.value;
  const exported = exportedIds.value;
  const audio = audioState.value;

  const isExported = exported.has(sentence.id);
  const hasPreview = preview && preview.sentenceId === sentence.id;
  const selectedWord = selected.get(sentence.id);

  const liClass = [
    isExported && 'exported',
    hasPreview && 'has-preview',
  ].filter(Boolean).join(' ');

  const isPlaying = audio.sentenceId === sentence.id && audio.playing;
  const isLoading = audio.sentenceId === sentence.id && audio.loading;

  function handleWordClick(baseForm) {
    if (isExported) return;

    // Toggle: click same word deselects
    if (preview && preview.sentenceId === sentence.id && preview.word === baseForm) {
      activePreview.value = null;
      const next = new Map(selected);
      next.delete(sentence.id);
      selectedWords.value = next;
      return;
    }

    // Select new word
    activePreview.value = { sentenceId: sentence.id, word: baseForm };
    const next = new Map(selected);
    next.set(sentence.id, baseForm);
    selectedWords.value = next;
  }

  function handlePlay() {
    // Dispatch custom event for audio-player to handle
    window.dispatchEvent(new CustomEvent('play-sentence', {
      detail: { videoId, sentenceId: sentence.id },
    }));
  }

  return html`
    <li class=${liClass}>
      <button
        class="play-btn ${isLoading ? 'loading' : ''}"
        onClick=${handlePlay}
        disabled=${isLoading}
        title="Play audio"
      >
        ${isPlaying ? '\u25A0' : isLoading ? '\u25CB' : '\u25B6'}
      </button>
      <span class="timestamp">${sentence.timestamp}</span>
      <span class="sentence-tokens">
        ${sentence.tokens.map((tok) => {
          if (!tok.is_content_word) {
            return html`<span class="token">${tok.surface}</span>`;
          }
          const isSelected = selectedWord === tok.base_form && hasPreview;
          return html`
            <span
              class="token content-word ${isSelected ? 'selected' : ''}"
              onClick=${() => handleWordClick(tok.base_form)}
            >${tok.surface}</span>
          `;
        })}
      </span>
    </li>
  `;
}
