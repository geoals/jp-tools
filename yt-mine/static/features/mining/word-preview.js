import { html } from 'htm/preact';
import { useState, useEffect, useRef } from 'preact/hooks';
import { fetchPreview, fetchLlmDefinition } from '../../api.js';

export function WordPreview({ videoId, sentenceId, word }) {
  const [preview, setPreview] = useState(null);
  const [llmDef, setLlmDef] = useState(null);
  const [llmLoading, setLlmLoading] = useState(false);
  const abortRef = useRef(null);

  useEffect(() => {
    setPreview(null);
    setLlmDef(null);
    setLlmLoading(true);

    // Abort previous LLM request
    if (abortRef.current) abortRef.current.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    // Fetch dictionary preview
    fetchPreview(videoId, sentenceId, word)
      .then(setPreview)
      .catch(() => {});

    // Fetch LLM definition (may take a while)
    fetchLlmDefinition(videoId, sentenceId, word, controller.signal)
      .then((data) => {
        setLlmDef(data.definition);
        setLlmLoading(false);
      })
      .catch((err) => {
        if (err.name !== 'AbortError') setLlmLoading(false);
      });

    return () => controller.abort();
  }, [videoId, sentenceId, word]);

  return html`
    <div class="preview-panel">
      ${preview && html`
        <div class="preview-header">
          <span class="preview-word">${preview.word}</span>
          ${preview.reading && html`<span class="preview-reading">${preview.reading}</span>`}
          ${preview.pitch_num && html`<span class="preview-pitch">[${preview.pitch_num}]</span>`}
        </div>
        ${preview.definition_html
          ? html`<div class="preview-definitions" dangerouslySetInnerHTML=${{ __html: preview.definition_html }}></div>`
          : html`<div class="preview-definitions preview-empty">No dictionary entries found.</div>`
        }
      `}
      <div class="preview-llm">
        ${llmLoading
          ? html`<span class="llm-spinner"></span>`
          : llmDef
            ? html`<div class="llm-definition">${llmDef}</div>`
            : html`<div class="llm-definition llm-unavailable">AI definition unavailable</div>`
        }
      </div>
    </div>
  `;
}
