import { render } from 'preact';
import { html } from 'htm/preact';
import { route, navigate } from './router.js';
import { QueuePage } from './features/mine/queue-page.js';
import { PhotoPage } from './features/mine/photo-page.js';
import { exportsPending, exportError } from './features/mine/state.js';

/** Floating status for fire-and-forget exports running in the background. */
function ExportToast() {
  const pending = exportsPending.value;
  const error = exportError.value;

  if (error) {
    return html`
      <div class="export-toast export-toast-error" onClick=${() => { exportError.value = null; }}>
        ${error} — photo kept in queue. (tap to dismiss)
      </div>
    `;
  }
  if (pending > 0) {
    return html`
      <div class="export-toast">
        <span class="spinner"></span> Exporting${pending > 1 ? ` (${pending})` : ''}…
      </div>
    `;
  }
  return null;
}

function App() {
  const { page, name } = route.value;

  return html`
    <h1><a href="/" onClick=${(e) => { e.preventDefault(); navigate('/'); }}>manga-mine</a></h1>
    ${page === 'queue' && html`<${QueuePage} />`}
    ${page === 'photo' && html`<${PhotoPage} key=${name} name=${name} />`}
    <${ExportToast} />
  `;
}

render(html`<${App} />`, document.getElementById('app'));
