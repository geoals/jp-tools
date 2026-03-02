import { render } from 'preact';
import { html } from 'htm/preact';
import { route, navigate } from './router.js';
import { SubmitForm } from './features/mining/submit-form.js';
import { VideoPage } from './features/mining/video-page.js';
import { AudioPlayer } from './features/mining/audio-player.js';
import { VocabPage } from './features/vocab/vocab-page.js';

function Nav() {
  return html`
    <nav class="site-nav">
      <a href="/" onClick=${(e) => { e.preventDefault(); navigate('/'); }}>Mining</a>
      <a href="/vocab" onClick=${(e) => { e.preventDefault(); navigate('/vocab'); }}>Vocab</a>
    </nav>
  `;
}

function App() {
  const { page, videoId } = route.value;

  return html`
    <h1><a href="/">yt-mine</a></h1>
    <${Nav} />
    ${page === 'home' && html`<${SubmitForm} />`}
    ${page === 'video' && html`<${VideoPage} videoId=${videoId} />`}
    ${page === 'vocab' && html`<${VocabPage} />`}
    <${AudioPlayer} />
  `;
}

render(html`<${App} />`, document.getElementById('app'));
