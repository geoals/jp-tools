import { html } from 'htm/preact';
import { activePopup, selectedWords, exportedIds, audioState, judged } from './state.js';

export function SentenceRow({ sentence, videoId, isTranscribing }) {
  const popup = activePopup.value;
  const selected = selectedWords.value;
  const exported = exportedIds.value;
  const audio = audioState.value;
  const marks = judged.value;

  const isExported = exported.has(sentence.id);
  const hasPopup = popup && popup.sentenceId === sentence.id;
  const selectedWord = selected.get(sentence.id);

  const liClass = [isExported && 'exported', hasPopup && 'has-popup']
    .filter(Boolean)
    .join(' ');

  const isPlaying = audio.sentenceId === sentence.id && audio.playing;
  const isLoading = audio.sentenceId === sentence.id && audio.loading;

  function handleWordClick(tok, event) {
    if (isExported) return;

    // Clicking the open word closes it, so one control both opens and dismisses.
    if (hasPopup && popup.target.term === tok.base_form && popup.target.reading === tok.reading) {
      activePopup.value = null;
      return;
    }

    activePopup.value = {
      sentenceId: sentence.id,
      rect: event.currentTarget.getBoundingClientRect(),
      target: {
        term: tok.base_form,
        // The ledger key and the dictionary's spelling are the same thing for
        // a token — they only diverge for a match picked out of the scan.
        key: tok.base_form,
        reading: tok.reading,
        surface: tok.surface,
        status: tok.status,
        start: tok.start,
      },
    };
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
        ${isPlaying ? '■' : isLoading ? '○' : '▶'}
      </button>
      <span class="timestamp">${sentence.timestamp}</span>
      ${selectedWord && html`<span class="card-word" title="This sentence's card word">${selectedWord.key}</span>`}
      <span class="sentence-tokens">
        ${sentence.tokens.map((tok) => {
          if (!tok.is_content_word) {
            return html`<span class="token">${tok.surface}</span>`;
          }
          // The tint is the ledger's, repainted from `judged` for anything
          // marked since the page loaded. `known` gets no tint at all — the
          // absence of a mark is what makes the marks readable, same as
          // read-stats' feed.
          const status = marks.get(`${tok.base_form} ${tok.reading}`) ?? tok.status;
          const isOpen = hasPopup
            && popup.target.term === tok.base_form
            && popup.target.reading === tok.reading;
          const cls = [
            'token content-word',
            status && status !== 'known' ? `mark-${status}` : '',
            selectedWord && selectedWord.key === tok.base_form ? 'selected' : '',
            isOpen ? 'open' : '',
          ].filter(Boolean).join(' ');
          return html`
            <span class=${cls} onClick=${(e) => handleWordClick(tok, e)}>${tok.surface}</span>
          `;
        })}
      </span>
    </li>
  `;
}
