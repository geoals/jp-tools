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

export async function fetchVideos() {
  const res = await request(`${BASE}/videos`);
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

export async function fetchDefine(term, reading) {
  const params = new URLSearchParams({ term });
  if (reading) params.set('reading', reading);
  const res = await request(`${BASE}/define?${params}`);
  return res.json();
}

// Every other reading of this position — the escape hatch for a word the
// tokenizer split, or read the other way. Never throws: an empty list is the
// common answer and the popup draws nothing for it.
export async function fetchExpand(text) {
  try {
    const res = await fetch(`${BASE}/expand?${new URLSearchParams({ text })}`);
    return res.ok ? await res.json() : [];
  } catch {
    return [];
  }
}

export async function judgeWord(headword, reading, status) {
  const res = await fetch(`${BASE}/judge`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ headword, reading, status }),
  });
  return res.ok;
}

// `words` is [{ headword, reading }] — ledger keys, as the tokens carry them.
export async function judgeWords(words, status) {
  const res = await fetch(`${BASE}/judge/many`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ words, status }),
  });
  return res.ok;
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
