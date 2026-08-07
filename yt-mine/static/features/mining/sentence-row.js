import { html } from 'htm/preact';
import { exportedIds, audioState, judged } from './state.js';
import { toggle, wheel } from './popup.js';

export function SentenceRow({ sentence, videoId, jobId }) {
  const exported = exportedIds.value;
  const audio = audioState.value;
  const marks = judged.value;

  const isExported = exported.has(sentence.id);
  const isPlaying = audio.sentenceId === sentence.id && audio.playing;
  const isLoading = audio.sentenceId === sentence.id && audio.loading;

  function open(tok, event) {
    if (isExported) return;
    // The click must not reach the document handler that dismisses.
    event.stopPropagation();
    toggle(
      event.currentTarget,
      {
        term: tok.base_form,
        // The ledger key and the dictionary's spelling are the same thing for
        // a token — they diverge only for a match picked out of the scan.
        key: tok.base_form,
        reading: tok.reading,
        surface: tok.surface,
        status: marks.get(`${tok.base_form} ${tok.reading}`) ?? tok.status,
        start: tok.start,
      },
      { videoId, jobId, sentenceId: sentence.id, text: sentence.text },
    );
  }

  function handlePlay() {
    // Dispatch custom event for audio-player to handle
    window.dispatchEvent(new CustomEvent('play-sentence', {
      detail: { videoId, sentenceId: sentence.id },
    }));
  }

  return html`
    <li class=${isExported ? 'exported' : ''}>
      <button
        class="play-btn ${isLoading ? 'loading' : ''}"
        onClick=${handlePlay}
        disabled=${isLoading}
        title="Play audio"
      >
        ${isPlaying ? '■' : isLoading ? '○' : '▶'}
      </button>
      <span class="timestamp">${sentence.timestamp}</span>
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
          const cls = ['token content-word', status && status !== 'known' ? `mark-${status}` : '']
            .filter(Boolean)
            .join(' ');
          return html`
            <span
              class=${cls}
              onClick=${(e) => open(tok, e)}
              onWheel=${(e) => wheel(e, e.currentTarget)}
            >${tok.surface}</span>
          `;
        })}
      </span>
    </li>
  `;
}
