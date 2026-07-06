import { html } from 'htm/preact';
import { useState, useEffect } from 'preact/hooks';
import { fetchPreview } from '../../api.js';

export function WordPreview({ word }) {
  const [preview, setPreview] = useState(null);

  useEffect(() => {
    setPreview(null);
    let cancelled = false;
    fetchPreview(word)
      .then((data) => { if (!cancelled) setPreview(data); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [word]);

  if (!preview) {
    return html`<div class="preview-panel"><span class="spinner"></span></div>`;
  }

  return html`
    <div class="preview-panel">
      <div class="preview-header">
        <span class="preview-word">${preview.word}</span>
        ${preview.reading && html`<span class="preview-reading">${preview.reading}</span>`}
        ${preview.pitch_num && html`<span class="preview-pitch">[${preview.pitch_num}]</span>`}
        ${preview.frequency != null && html`<span class="preview-freq">${preview.frequency}</span>`}
      </div>
      ${preview.definition_html
        ? html`<div class="preview-definitions" dangerouslySetInnerHTML=${{ __html: preview.definition_html }}></div>`
        : html`<div class="preview-definitions preview-empty">No dictionary entries found.</div>`
      }
    </div>
  `;
}
