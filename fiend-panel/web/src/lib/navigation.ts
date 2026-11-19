import type { SessionInfo } from './api/generated';

export type PanelUrl = Pick<URL, 'pathname' | 'search' | 'hash' | 'origin'> & {
  searchParams: Pick<URLSearchParams, 'get'>;
};

export function panelPath(url: PanelUrl): string {
  return url.pathname + url.search + url.hash;
}

export function isOpenPath(path: string): boolean {
  return path === '/login' || path === '/login/' || path === '/setup' || path.startsWith('/setup/');
}

export function loginPath(url: PanelUrl): string {
  return `/login?next=${encodeURIComponent(panelPath(url))}`;
}

export function returnPath(url: PanelUrl): string {
  const next = url.searchParams.get('next');
  if (!next?.startsWith('/')) return '/';
  try {
    const target = new URL(next, url.origin);
    return target.origin === url.origin && !isOpenPath(target.pathname) ? panelPath(target) : '/';
  } catch {
    return '/';
  }
}

export function signedInPath(info: SessionInfo, url: PanelUrl, next: string): string {
  if (info.wizard.done || new URL(next, url.origin).pathname === '/welcome') return next;
  return next === '/' ? '/welcome' : `/welcome?next=${encodeURIComponent(next)}`;
}
