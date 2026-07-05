import { signal } from '@preact/signals';

export const route = signal(parseRoute());

function parseRoute() {
  const path = window.location.pathname;

  if (path === '/' || path === '') {
    return { page: 'queue' };
  }

  // /p/{name} — photo mining page
  if (path.startsWith('/p/')) {
    const name = decodeURIComponent(path.slice(3));
    if (name && !name.includes('/')) {
      return { page: 'photo', name };
    }
  }

  return { page: 'queue' };
}

export function navigate(path) {
  window.history.pushState(null, '', path);
  route.value = parseRoute();
}

window.addEventListener('popstate', () => {
  route.value = parseRoute();
});
