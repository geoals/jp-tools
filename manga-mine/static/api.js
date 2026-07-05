const BASE = '/api';

async function request(url, options = {}) {
  const res = await fetch(url, options);
  if (!res.ok && res.status !== 204) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  return res;
}

export async function fetchQueue() {
  const res = await request(`${BASE}/queue`);
  return res.json();
}

export async function uploadPhoto(file) {
  const form = new FormData();
  form.append('photo', file, file.name);
  const res = await request(`${BASE}/photos`, { method: 'POST', body: form });
  return res.json();
}

export function photoUrl(name) {
  return `${BASE}/photos/${encodeURIComponent(name)}`;
}

export function thumbUrl(name) {
  return `${BASE}/photos/${encodeURIComponent(name)}/thumb`;
}

export async function ocrCrop(name, rect) {
  const res = await request(`${BASE}/photos/${encodeURIComponent(name)}/ocr`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(rect),
  });
  return res.json();
}

export async function fetchPreview(word) {
  const params = new URLSearchParams({ word });
  const res = await request(`${BASE}/preview?${params}`);
  return res.json();
}

export async function exportCard(photo, sentence, targetWord) {
  const res = await fetch(`${BASE}/export`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ photo, sentence, target_word: targetWord }),
  });
  if (!res.ok) {
    let message = 'Export failed';
    try {
      const body = await res.json();
      message = body.error || message;
    } catch { /* not json */ }
    throw new Error(message);
  }
  return res.json();
}

export async function markPhoto(name, status) {
  await request(`${BASE}/photos/${encodeURIComponent(name)}/mark`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ status }),
  });
}
