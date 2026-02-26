import { html } from 'https://esm.sh/htm@3.1.1/preact/standalone';
import { useEffect, useRef } from 'https://esm.sh/preact@10.25.4/hooks';
import { audioState } from '../state.js';

export function AudioPlayer() {
  const audioRef = useRef(null);

  useEffect(() => {
    function handlePlay(e) {
      const { videoId, sentenceId } = e.detail;
      const audio = audioRef.current;
      const current = audioState.value;

      // Toggle off if same sentence is playing
      if (current.sentenceId === sentenceId && current.playing) {
        audio.pause();
        audio.currentTime = 0;
        audioState.value = { playing: false, loading: false, sentenceId: null };
        return;
      }

      // Play new sentence
      audioState.value = { playing: false, loading: true, sentenceId };
      audio.src = `/${videoId}/sentences/${sentenceId}/audio`;
      audio.load();
      audio.play()
        .then(() => {
          audioState.value = { playing: true, loading: false, sentenceId };
        })
        .catch(() => {
          audioState.value = { playing: false, loading: false, sentenceId: null };
        });
    }

    window.addEventListener('play-sentence', handlePlay);
    return () => window.removeEventListener('play-sentence', handlePlay);
  }, []);

  function handleEnded() {
    audioState.value = { playing: false, loading: false, sentenceId: null };
  }

  return html`<audio ref=${audioRef} onEnded=${handleEnded} style="display:none" />`;
}
