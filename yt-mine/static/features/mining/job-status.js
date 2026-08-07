import { html } from 'htm/preact';

export function JobStatus({ status, errorMessage, progressPercent, refineState, refineAt }) {
  const isDone = status === 'done';
  const isError = status === 'error';
  const refining = refineState === 'running';
  const refineFailed = refineState && refineState !== 'running' && refineState !== 'done';

  let text;
  if (isError) {
    text = errorMessage || 'Something went wrong.';
  } else if (refining) {
    text = `Sharpening ${formatSeconds(refineAt)} with whisper...`;
  } else if (refineFailed) {
    text = refineState;
  } else if (isDone) {
    text = 'Click a word to look it up. ⟳ re-transcribes a line with whisper.';
  } else if (status === 'transcribing') {
    const pct = progressPercent != null ? ` (${progressPercent}%)` : '';
    text = `No captions for this video — transcribing it${pct}...`;
  } else if (status === 'downloading') {
    text = 'No captions for this video — downloading the audio...';
  } else if (status === 'fetching') {
    text = 'Reading the video’s captions...';
  } else {
    text = 'Pending...';
  }

  const statusClass = refineFailed ? 'error' : isError ? 'error' : isDone && !refining ? 'done' : '';

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
  if (secs == null) return '';
  const total = Math.floor(secs);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}
