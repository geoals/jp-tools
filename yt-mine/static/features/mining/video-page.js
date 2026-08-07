import { html } from 'htm/preact';
import { useState, useEffect, useRef } from 'preact/hooks';
import { fetchJob, pollStatus } from '../../api.js';
import { JobStatus } from './job-status.js';
import { SentenceList } from './sentence-list.js';

export function VideoPage({ videoId, at }) {
  const [job, setJob] = useState(null);
  const [error, setError] = useState(null);
  const pollRef = useRef(null);
  // The list arrives one fetch after the page does, and grows while whisper
  // runs, so landing on the line is a one-shot that waits for it to exist.
  const landed = useRef(false);

  // Initial fetch
  useEffect(() => {
    let cancelled = false;
    fetchJob(videoId)
      .then((data) => { if (!cancelled) setJob(data); })
      .catch((err) => { if (!cancelled) setError(err.message); });
    return () => { cancelled = true; };
  }, [videoId]);

  // Polling
  useEffect(() => {
    if (!job || job.is_terminal) return;

    const controller = new AbortController();
    pollRef.current = setInterval(async () => {
      try {
        const data = await pollStatus(videoId, job.sentence_count, job.status);
        if (data) setJob(data);
      } catch (_) {
        // Ignore transient poll errors
      }
    }, 2000);

    return () => {
      clearInterval(pollRef.current);
      controller.abort();
    };
  }, [job?.status, job?.sentence_count, videoId]);

  // Whisper transcribes from 0:00, so the line a `t=` link points at may not
  // be there yet. Every poll is another chance to land on it.
  useEffect(() => {
    if (!at || landed.current || !job?.sentences?.length) return;
    if (!job.is_terminal && lastStart(job.sentences) < at) return;
    landed.current = true;
    scrollToTime(at);
  }, [at, job?.sentence_count, job?.is_terminal]);

  if (error) {
    return html`<div class="status error"><span class="progress-text">${error}</span></div>`;
  }

  if (!job) {
    return html`<div class="status"><span class="progress-text">Loading...</span></div>`;
  }

  const isDone = job.status === 'done';
  const isTranscribing = job.status === 'transcribing';

  return html`
    ${job.video_title && html`<h2>${job.video_title}</h2>`}
    <${JobStatus}
      status=${job.status}
      errorMessage=${job.error_message}
      progressPercent=${job.progress_percent}
    />
    <${SentenceList}
      sentences=${job.sentences}
      videoId=${videoId}
      jobId=${job.job_id}
      isDone=${isDone}
      isTranscribing=${isTranscribing}
    />
  `;
}

function lastStart(sentences) {
  return sentences[sentences.length - 1]?.start_seconds ?? 0;
}

// The first line at or after `seconds`, centred and outlined so it is
// findable in a list of several hundred.
function scrollToTime(seconds) {
  requestAnimationFrame(() => {
    const rows = [...document.querySelectorAll('.sentence-list > li[data-start]')];
    const row = rows.find((r) => Number(r.dataset.start) >= seconds - 1) ?? rows.at(-1);
    if (!row) return;
    row.scrollIntoView({ block: 'center' });
    row.classList.add('landed');
  });
}
