import { signal } from '@preact/signals';

export const route = signal(parseRoute());

function parseRoute() {
  const path = window.location.pathname;

  if (path === '/' || path === '') {
    return { page: 'home' };
  }

  // /{videoId} — everything after the leading slash
  const videoId = path.slice(1);
  if (videoId && !videoId.includes('/')) {
    // `?t=` is where the page opens. It survives a reload and a copied link,
    // which a signal would not.
    const t = Number(new URLSearchParams(window.location.search).get('t'));
    return { page: 'video', videoId, at: Number.isFinite(t) && t > 0 ? t : null };
  }

  return { page: 'home' };
}

export function navigate(path) {
  window.history.pushState(null, '', path);
  route.value = parseRoute();
}

window.addEventListener('popstate', () => {
  route.value = parseRoute();
});
