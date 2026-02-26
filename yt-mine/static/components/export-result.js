import { html } from 'https://esm.sh/htm@3.1.1/preact/standalone';
import { exportResult } from '../state.js';

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
