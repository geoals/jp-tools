import { signal } from '@preact/signals';

export const route = signal(parseRoute());

function parseRoute() {
  const path = window.location.pathname;

  if (path === '/' || path === '') {
    return { page: 'home' };
  }

  if (path === '/vocab') {
    return { page: 'vocab' };
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
