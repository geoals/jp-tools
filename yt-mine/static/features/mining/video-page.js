import { html } from 'htm/preact';
import { useState, useEffect, useRef } from 'preact/hooks';
import { fetchJob, pollStatus, refineAt } from '../../api.js';
import { JobStatus } from './job-status.js';
import { SentenceList } from './sentence-list.js';

export function VideoPage({ videoId, at }) {
  const [job, setJob] = useState(null);
  const [error, setError] = useState(null);
  const pollRef = useRef(null);
  // The line the page opened on, and whether it has been scrolled to yet —
  // the list arrives one fetch after the page does.
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
    if (!job) return;
    // A sharpening window keeps the page live after the job itself is done.
    if (job.is_terminal && job.refine_state !== 'running') return;

    pollRef.current = setInterval(async () => {
      try {
        const data = await pollStatus(
          videoId,
          job.sentence_count,
          job.status,
          job.refine_state ?? '',
        );
        if (data) setJob(data);
      } catch (_) {
        // Ignore transient poll errors
      }
    }, 2000);

    return () => clearInterval(pollRef.current);
  }, [job?.status, job?.sentence_count, job?.refine_state, videoId]);

  // Open on the line that was being watched, and sharpen it without being
  // asked: arriving here from "Copy link at current time" means a card is
  // about to be made from that line.
  useEffect(() => {
    if (!at || landed.current || !job?.sentences?.length) return;
    landed.current = true;
    scrollToTime(at);
    if (job.is_terminal && job.refine_state !== 'running') {
      refineAt(videoId, at).then((ok) => {
        if (ok) setJob((j) => ({ ...j, refine_state: 'running', refine_at: at }));
      });
    }
  }, [at, job?.sentences?.length]);

  async function sharpen(seconds) {
    if (await refineAt(videoId, seconds)) {
      setJob((j) => ({ ...j, refine_state: 'running', refine_at: seconds }));
    }
  }

  if (error) {
    return html`<div class="status error"><span class="progress-text">${error}</span></div>`;
  }

  if (!job) {
    return html`<div class="status"><span class="progress-text">Loading...</span></div>`;
  }

  return html`
    ${job.video_title && html`<h2>${job.video_title}</h2>`}
    <${JobStatus}
      status=${job.status}
      errorMessage=${job.error_message}
      progressPercent=${job.progress_percent}
      refineState=${job.refine_state}
      refineAt=${job.refine_at}
    />
    <${SentenceList}
      sentences=${job.sentences}
      videoId=${videoId}
      jobId=${job.job_id}
      isTranscribing=${job.status === 'transcribing'}
      openedAt=${at}
      refining=${job.refine_state === 'running' ? job.refine_at : null}
      onSharpen=${sharpen}
    />
  `;
}

// The first line at or after `seconds`, centred and flashed so it is
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
