const BASE = '/api';

async function request(url, options = {}) {
  const res = await fetch(url, options);
  if (!res.ok && res.status !== 204) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  return res;
}

export async function submitUrl(url) {
  const res = await request(`${BASE}/jobs`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ url }),
  });
  return res.json();
}

export async function fetchJob(videoId) {
  const res = await request(`${BASE}/${videoId}`);
  return res.json();
}

export async function pollStatus(videoId, sentenceCount, status) {
  const params = new URLSearchParams();
  if (sentenceCount != null) params.set('sc', sentenceCount);
  if (status != null) params.set('st', status);
  const res = await fetch(`${BASE}/${videoId}/status?${params}`);
  if (res.status === 204) return null;
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function fetchPreview(videoId, sentenceId, word) {
  const params = new URLSearchParams({ word });
  const res = await request(
    `${BASE}/${videoId}/sentences/${sentenceId}/preview?${params}`
  );
  return res.json();
}

export async function fetchLlmDefinition(videoId, sentenceId, word, signal) {
  const params = new URLSearchParams({ word });
  const res = await fetch(
    `${BASE}/${videoId}/sentences/${sentenceId}/llm-definition?${params}`,
    { signal }
  );
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

export async function exportSentences(jobId, sentences) {
  const res = await fetch(`${BASE}/export`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ job_id: jobId, sentences }),
  });
  const body = await res.json();
  if (!res.ok) throw new Error(body.error || 'Export failed');
  return body;
}
