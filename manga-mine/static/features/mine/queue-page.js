import { html } from 'htm/preact';
import { useState, useEffect, useRef } from 'preact/hooks';
import { fetchQueue, uploadPhoto, thumbUrl } from '../../api.js';
import { navigate } from '../../router.js';
import { queueVersion } from './state.js';

export function QueuePage() {
  const [photos, setPhotos] = useState(null);
  const [error, setError] = useState(null);
  const [uploading, setUploading] = useState(false);
  const fileRef = useRef(null);

  async function refresh() {
    try {
      const data = await fetchQueue();
      setPhotos(data.photos);
      setError(null);
    } catch (e) {
      setError(e.message);
    }
  }

  // Re-fetch when a background export completes (the exported photo is
  // deleted only once the export succeeds)
  useEffect(() => { refresh(); }, [queueVersion.value]);

  async function handleFiles(e) {
    const files = Array.from(e.target.files || []);
    if (!files.length) return;
    setUploading(true);
    try {
      for (const file of files) {
        await uploadPhoto(file);
      }
      await refresh();
    } catch (err) {
      setError(err.message);
    } finally {
      setUploading(false);
      if (fileRef.current) fileRef.current.value = '';
    }
  }

  return html`
    <div class="queue-page">
      <div class="queue-header">
        <p class="helper-text">
          ${photos == null ? 'Loading queue…'
            : photos.length === 0 ? 'Inbox is empty — synced photos show up here.'
            : `${photos.length} photo${photos.length === 1 ? '' : 's'} to mine`}
        </p>
        <label class="upload-btn">
          ${uploading ? html`<span class="spinner"></span>` : '+ Upload'}
          <input
            ref=${fileRef}
            type="file"
            accept="image/*"
            multiple
            hidden
            onChange=${handleFiles}
          />
        </label>
      </div>
      ${error && html`<div class="error-banner">${error}</div>`}
      ${photos != null && photos.length > 0 && html`
        <div class="queue-grid">
          ${photos.map((p) => html`
            <button
              key=${p.name}
              class="queue-item"
              onClick=${() => navigate(`/p/${encodeURIComponent(p.name)}`)}
            >
              <img src=${thumbUrl(p.name)} alt=${p.name} loading="lazy" />
              <span class="queue-item-name">${p.name}</span>
            </button>
          `)}
        </div>
      `}
    </div>
  `;
}
