import { html } from 'htm/preact';
import { useState } from 'preact/hooks';
import { submitUrl } from '../../api.js';
import { navigate } from '../../router.js';

export function SubmitForm() {
  const [url, setUrl] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState(null);

  async function handleSubmit(e) {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      const { video_id, at } = await submitUrl(url);
      navigate(at ? `/${video_id}?t=${Math.round(at)}` : `/${video_id}`);
    } catch (err) {
      setError(err.message);
      setSubmitting(false);
    }
  }

  return html`
    <form onSubmit=${handleSubmit}>
      <input
        type="url"
        name="url"
        placeholder="https://www.youtube.com/watch?v=..."
        value=${url}
        onInput=${(e) => setUrl(e.target.value)}
        required
      />
      <p class="helper-text">
        Right-click the video → <b>Copy link at current time</b>, and the page
        opens on the line you were watching.
      </p>
      <button type="submit" disabled=${submitting}>
        ${submitting && html`<span class="spinner"></span>`}
        <span>Mine sentences</span>
      </button>
      ${error && html`<p class="helper-text" style="color: var(--danger-text)">${error}</p>`}
    </form>
  `;
}
