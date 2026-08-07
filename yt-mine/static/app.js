import { render } from 'preact';
import { html } from 'htm/preact';
import { route, navigate } from './router.js';
import { SubmitForm } from './features/mining/submit-form.js';
import { VideoPage } from './features/mining/video-page.js';
import { AudioPlayer } from './features/mining/audio-player.js';

function App() {
  const { page, videoId, at } = route.value;

  return html`
    <h1><a href="/">yt-mine</a></h1>
    ${page === 'home' && html`<${SubmitForm} />`}
    ${page === 'video' && html`<${VideoPage} videoId=${videoId} at=${at} />`}
    <${AudioPlayer} />
  `;
}

render(html`<${App} />`, document.getElementById('app'));
