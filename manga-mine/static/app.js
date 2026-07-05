import { render } from 'preact';
import { html } from 'htm/preact';
import { route, navigate } from './router.js';
import { QueuePage } from './features/mine/queue-page.js';
import { PhotoPage } from './features/mine/photo-page.js';

function App() {
  const { page, name } = route.value;

  return html`
    <h1><a href="/" onClick=${(e) => { e.preventDefault(); navigate('/'); }}>manga-mine</a></h1>
    ${page === 'queue' && html`<${QueuePage} />`}
    ${page === 'photo' && html`<${PhotoPage} key=${name} name=${name} />`}
  `;
}

render(html`<${App} />`, document.getElementById('app'));
