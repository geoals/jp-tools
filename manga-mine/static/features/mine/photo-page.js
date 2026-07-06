import { html } from 'htm/preact';
import { useState, useRef, useEffect } from 'preact/hooks';
import { photoUrl, ocrCrop, exportCard, markPhoto, fetchQueue, fetchSources } from '../../api.js';
import { navigate } from '../../router.js';
import { CropBox } from './crop-box.js';
import { WordPreview } from './word-preview.js';
import { exportsPending, exportError, queueVersion } from './state.js';

export function PhotoPage({ name }) {
  const [rect, setRect] = useState(null);          // crop in displayed px
  const [ocr, setOcr] = useState(null);            // { text, sentences }
  const [ocrLoading, setOcrLoading] = useState(false);
  const [selection, setSelection] = useState(null); // { sentenceIdx, word }
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [source, setSource] = useState('');
  const [knownSources, setKnownSources] = useState([]);
  const containerRef = useRef(null);
  const errorRef = useRef(null);
  const resultRef = useRef(null);

  // OCR result = crop locked in; bring the sentences to the top of the
  // screen so the word list and definition have room without scrolling.
  useEffect(() => {
    if (ocr && resultRef.current) {
      resultRef.current.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }
  }, [ocr]);

  // Remembered manga titles; most recent is preselected
  useEffect(() => {
    fetchSources()
      .then((data) => {
        setKnownSources(data.sources);
        if (data.sources.length > 0) setSource(data.sources[0]);
      })
      .catch(() => {});
  }, []);

  // The user may be scrolled to the export bar when an error appears higher
  // up the page — bring it into view.
  useEffect(() => {
    if (error && errorRef.current) {
      errorRef.current.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
  }, [error]);

  async function goNext() {
    try {
      const data = await fetchQueue();
      const next = data.photos.find((p) => p.name !== name);
      navigate(next ? `/p/${encodeURIComponent(next.name)}` : '/');
    } catch {
      navigate('/');
    }
  }

  // Runs automatically when the crop drag is released
  async function runOcr(box) {
    const el = containerRef.current;
    const r = box || rect;
    if (!r || !r.w || !el) return;
    setOcrLoading(true);
    setError(null);
    setSelection(null);
    try {
      const fractions = {
        x: r.x / el.clientWidth,
        y: r.y / el.clientHeight,
        w: r.w / el.clientWidth,
        h: r.h / el.clientHeight,
      };
      setOcr(await ocrCrop(name, fractions));
    } catch (e) {
      setError(e.message);
    } finally {
      setOcrLoading(false);
    }
  }

  function handleWordTap(sentenceIdx, baseForm) {
    if (selection && selection.sentenceIdx === sentenceIdx && selection.word === baseForm) {
      setSelection(null); // tap again to deselect
    } else {
      setSelection({ sentenceIdx, word: baseForm });
    }
  }

  // Fire-and-forget: move on immediately, let the Anki work (compress +
  // media upload + addNote, ~1.5s to a phone) finish in the background.
  // The photo is only deleted after the export succeeds, so a failure
  // leaves it in the queue to redo.
  function handleExport() {
    if (!selection) return;
    const word = selection.word;
    const sentence = ocr.sentences[selection.sentenceIdx].text;

    exportsPending.value += 1;
    exportCard(name, sentence, word, source.trim() || null)
      .then(() => markPhoto(name, 'processed'))
      .then(() => {
        exportsPending.value -= 1;
        queueVersion.value += 1;
      })
      .catch((e) => {
        exportsPending.value -= 1;
        exportError.value = `${word}: ${e.message}`;
      });

    goNext();
  }

  async function handleSkip() {
    setBusy(true);
    setError(null);
    try {
      await markPhoto(name, 'skipped');
      await goNext();
    } catch (e) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  }

  return html`
    <div class="photo-page">
      <p class="helper-text">
        ${ocr == null
          ? (ocrLoading ? 'Reading…' : 'Drag a box around the text — OCR runs when you let go.')
          : 'Tap the word you want to mine.'}
      </p>

      <${CropBox}
        src=${photoUrl(name)}
        rect=${rect}
        setRect=${setRect}
        onRelease=${runOcr}
        disabled=${ocrLoading || busy || ocr != null}
        containerRef=${containerRef}
      />

      <div class="photo-actions">
        ${ocrLoading && html`
          <button disabled><span class="spinner"></span> Reading…</button>
        `}
        ${ocr != null && html`
          <button class="secondary" onClick=${() => { setOcr(null); setSelection(null); }} disabled=${busy}>
            Re-crop
          </button>
        `}
        <button class="secondary" onClick=${handleSkip} disabled=${busy || ocrLoading}>
          Skip photo
        </button>
      </div>

      ${error && html`<div ref=${errorRef} class="error-banner">${error}</div>`}

      ${ocr && html`
        <div class="ocr-result" ref=${resultRef}>
          ${ocr.sentences.length === 0 && html`
            <p class="helper-text">No text recognized — try a tighter crop.</p>
          `}
          <ul class="sentence-list">
            ${ocr.sentences.map((sentence, si) => html`
              <li key=${si} class=${selection && selection.sentenceIdx === si ? 'has-preview' : ''}>
                <span class="sentence-tokens">
                  ${sentence.tokens.map((tok) => {
                    if (!tok.is_content_word) {
                      return html`<span class="token">${tok.surface}</span>`;
                    }
                    const isSelected = selection
                      && selection.sentenceIdx === si
                      && selection.word === tok.base_form;
                    return html`
                      <span
                        class="token content-word ${isSelected ? 'selected' : ''}"
                        onClick=${() => handleWordTap(si, tok.base_form)}
                      >${tok.surface}</span>
                    `;
                  })}
                </span>
                ${selection && selection.sentenceIdx === si && html`
                  <${WordPreview} word=${selection.word} />
                `}
              </li>
            `)}
          </ul>

          <div class="source-row">
            <label class="source-label" for="source-input">Source</label>
            <input
              id="source-input"
              type="text"
              list="source-suggestions"
              placeholder="Manga title"
              value=${source}
              onInput=${(e) => setSource(e.target.value)}
            />
            <datalist id="source-suggestions">
              ${knownSources.map((s) => html`<option key=${s} value=${s} />`)}
            </datalist>
          </div>

          <div class="export-bar">
            <button onClick=${handleExport} disabled=${!selection || busy}>
              Export to Anki
            </button>
            ${selection && html`
              <span class="export-hint">
                ${selection.word} — ${ocr.sentences[selection.sentenceIdx].text}
              </span>
            `}
          </div>
        </div>
      `}
    </div>
  `;
}
