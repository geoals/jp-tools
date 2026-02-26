import { html } from 'https://esm.sh/htm@3.1.1/preact/standalone';
import { useState } from 'https://esm.sh/preact@10.25.4/hooks';
import { submitUrl } from '../api.js';
import { navigate } from '../router.js';

export function SubmitForm() {
  const [url, setUrl] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState(null);

  async function handleSubmit(e) {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      const { video_id } = await submitUrl(url);
      navigate(`/${video_id}`);
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
      <button type="submit" disabled=${submitting}>
        ${submitting && html`<span class="spinner"></span>`}
        <span>Mine sentences</span>
      </button>
      ${error && html`<p class="helper-text" style="color: var(--danger-text)">${error}</p>`}
    </form>
  `;
}
