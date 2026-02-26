import { html, render } from 'https://esm.sh/htm@3.1.1/preact/standalone';
import { route } from './router.js';
import { SubmitForm } from './components/submit-form.js';
import { VideoPage } from './components/video-page.js';
import { AudioPlayer } from './components/audio-player.js';

function App() {
  const { page, videoId } = route.value;

  return html`
    <h1><a href="/">yt-mine</a></h1>
    <p><strong>YouTube sentence mining</strong></p>
    ${page === 'home' && html`<${SubmitForm} />`}
    ${page === 'video' && html`<${VideoPage} videoId=${videoId} />`}
    <${AudioPlayer} />
  `;
}

render(html`<${App} />`, document.getElementById('app'));
