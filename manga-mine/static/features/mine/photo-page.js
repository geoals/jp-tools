import { html } from 'htm/preact';
import { useState, useRef, useEffect } from 'preact/hooks';
import { photoUrl, ocrCrop, exportCard, markPhoto, fetchQueue, fetchSources } from '../../api.js';
import { navigate } from '../../router.js';
import { CropBox } from './crop-box.js';
import { WordPreview } from './word-preview.js';

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

  async function runOcr() {
    const el = containerRef.current;
    if (!rect || !el) return;
    setOcrLoading(true);
    setError(null);
    setSelection(null);
    try {
      const fractions = {
        x: rect.x / el.clientWidth,
        y: rect.y / el.clientHeight,
        w: rect.w / el.clientWidth,
        h: rect.h / el.clientHeight,
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

  async function handleExport() {
    if (!selection) return;
    setBusy(true);
    setError(null);
    try {
      const sentence = ocr.sentences[selection.sentenceIdx].text;
      await exportCard(name, sentence, selection.word, source.trim() || null);
      await markPhoto(name, 'processed');
      await goNext();
    } catch (e) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
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
          ? 'Drag a box around the text, then run OCR.'
          : 'Tap the word you want to mine.'}
      </p>

      <${CropBox}
        src=${photoUrl(name)}
        rect=${rect}
        setRect=${(r) => { setRect(r); setOcr(null); setSelection(null); }}
        disabled=${ocrLoading || busy}
        containerRef=${containerRef}
      />

      <div class="photo-actions">
        <button onClick=${runOcr} disabled=${!rect || !rect.w || ocrLoading || busy}>
          ${ocrLoading ? html`<span class="spinner"></span> Reading…` : 'Run OCR'}
        </button>
        <button class="secondary" onClick=${handleSkip} disabled=${busy || ocrLoading}>
          Skip photo
        </button>
      </div>

      ${error && html`<div ref=${errorRef} class="error-banner">${error}</div>`}

      ${ocr && html`
        <div class="ocr-result">
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
              ${busy ? html`<span class="spinner"></span> Exporting…` : 'Export to Anki'}
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
