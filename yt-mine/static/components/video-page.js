import { html } from 'https://esm.sh/htm@3.1.1/preact/standalone';
import { useState, useEffect, useRef } from 'https://esm.sh/preact@10.25.4/hooks';
import { fetchJob, pollStatus } from '../api.js';
import { JobStatus } from './job-status.js';
import { SentenceList } from './sentence-list.js';

export function VideoPage({ videoId }) {
  const [job, setJob] = useState(null);
  const [error, setError] = useState(null);
  const pollRef = useRef(null);

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
