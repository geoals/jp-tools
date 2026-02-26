import { html } from 'htm/preact';

export function JobStatus({ status, errorMessage, progressPercent }) {
  const isDone = status === 'done';
  const isError = status === 'error';
  const statusClass = isDone ? 'done' : isError ? 'error' : '';

  let text;
  if (isError) {
    text = errorMessage || 'Something went wrong.';
  } else if (isDone) {
    text = 'Done — select words and export to Anki';
  } else if (status === 'transcribing') {
    const pct = progressPercent != null ? ` (${progressPercent}%)` : '';
    text = `Transcribing${pct}...`;
  } else if (status === 'downloading') {
    text = 'Downloading audio...';
  } else {
    text = 'Pending...';
  }

  const fillWidth = status === 'transcribing' && progressPercent != null
    ? `${progressPercent}%`
    : '0%';

  return html`
    <div class="status ${statusClass}">
      ${status === 'transcribing' && html`
        <div class="progress-fill" style="width: ${fillWidth}"></div>
      `}
      <span class="progress-text">${text}</span>
    </div>
  `;
}
