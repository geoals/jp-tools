import { html } from 'htm/preact';
import { exportResult } from './state.js';

export function ExportResult() {
  const result = exportResult.value;
  if (!result) return null;

  const isError = result.startsWith('Error:');

  return html`
    <div id="export-result">
      <p class=${isError ? 'error' : ''}>${result}</p>
    </div>
  `;
}
