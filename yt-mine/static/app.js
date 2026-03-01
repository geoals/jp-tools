import { render } from 'preact';
import { html } from 'htm/preact';
import { route, navigate } from './router.js';
import { SubmitForm } from './components/submit-form.js';
import { VideoPage } from './components/video-page.js';
import { AudioPlayer } from './components/audio-player.js';
import { VocabPage } from './components/vocab-page.js';

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
