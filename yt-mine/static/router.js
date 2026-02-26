import { signal } from 'https://esm.sh/@preact/signals@1.3.1?deps=preact@10.25.4';

export const route = signal(parseRoute());

function parseRoute() {
  const path = window.location.pathname;

  if (path === '/' || path === '') {
    return { page: 'home' };
  }

  // /{videoId} — everything after the leading slash
  const videoId = path.slice(1);
  if (videoId && !videoId.includes('/')) {
    return { page: 'video', videoId };
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
