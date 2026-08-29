import { html } from 'htm/preact';
import { useState, useEffect } from 'preact/hooks';
import { fetchVideos } from '../../api.js';
import { navigate } from '../../router.js';

// The videos already processed. A transcript is worth coming back to: the words
// skipped the first time through are still in it.
export function VideoHistory() {
  const [videos, setVideos] = useState(null);

  useEffect(() => {
    let cancelled = false;
    fetchVideos()
      .then((data) => { if (!cancelled) setVideos(data); })
      .catch(() => { if (!cancelled) setVideos([]); });
    return () => { cancelled = true; };
  }, []);

  if (!videos?.length) return null;

  return html`
    <h2 class="history-heading">Processed</h2>
    <ul class="video-history">
      ${videos.map((v) => html`
        <li key=${v.video_id}>
          <a
            href=${`/${v.video_id}`}
            onClick=${(e) => { e.preventDefault(); navigate(`/${v.video_id}`); }}
          >${v.video_title || v.video_id}</a>
          <span class="meta">${meta(v)}</span>
        </li>
      `)}
    </ul>
  `;
}

function meta(v) {
  const lines = v.status === 'error' ? 'failed' : `${v.sentence_count} lines`;
  return `${lines} · ${day(v.created_at)}`;
}

// `created_at` is unix seconds, written as a string.
function day(created) {
  const secs = Number(created);
  if (!Number.isFinite(secs)) return created;
  return new Date(secs * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}
