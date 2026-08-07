import { html } from 'htm/preact';

export function JobStatus({ status, errorMessage, progressPercent, waitingFor }) {
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
    // The link's timestamp is worth naming: it is the only thing on screen
    // that says why the page will jump on its own later.
    const target = waitingFor != null ? `, jumping to ${formatSeconds(waitingFor)} when it gets there` : '';
    text = `Transcribing${pct}${target}...`;
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

function formatSeconds(secs) {
  const total = Math.floor(secs);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}
