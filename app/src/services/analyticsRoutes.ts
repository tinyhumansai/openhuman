/** Convert a HashRouter hash into its application path. */
export function hashToPath(hash: string): string {
  const withoutHash = hash.slice(1);
  return withoutHash || '/';
}

/** Normalize dynamic application paths into privacy-safe analytics templates. */
export function normalizeAnalyticsPagePath(path: string): string {
  const rawPath =
    typeof window !== 'undefined' && window.location.hash.startsWith('#/')
      ? hashToPath(window.location.hash)
      : path.startsWith('#/')
        ? hashToPath(path)
        : path || '/';

  // Analytics records route templates, never entity identifiers or query/hash
  // values. Besides avoiding high-cardinality dashboards, this prevents thread,
  // flow, profile, and team identifiers from leaving the app.
  const pathname = rawPath.split(/[?#]/, 1)[0] || '/';
  if (/^\/chat\/[^/]+/.test(pathname)) return '/chat/:threadId';
  if (/^\/flows\/[^/]+/.test(pathname) && pathname !== '/flows/draft') {
    return '/flows/:flowId';
  }
  if (/^\/settings\/team\/manage\/[^/]+\/members$/.test(pathname)) {
    return '/settings/team/manage/:teamId/members';
  }
  if (/^\/settings\/team\/manage\/[^/]+\/invites$/.test(pathname)) {
    return '/settings/team/manage/:teamId/invites';
  }
  if (/^\/settings\/team\/manage\/[^/]+$/.test(pathname)) {
    return '/settings/team/manage/:teamId';
  }
  if (/^\/settings\/agents\/edit\/[^/]+$/.test(pathname)) {
    return '/settings/agents/edit/:id';
  }
  if (/^\/settings\/profiles\/edit\/[^/]+$/.test(pathname)) {
    return '/settings/profiles/edit/:id';
  }
  if (/^\/callback\/[^/]+\/[^/]+$/.test(pathname)) return '/callback/:kind/:status';
  if (/^\/callback\/[^/]+$/.test(pathname)) return '/callback/:kind';
  return pathname;
}

/** Read the current privacy-normalized application path. */
export function currentAppPath(): string {
  if (typeof window === 'undefined') return '';
  return normalizeAnalyticsPagePath(window.location.pathname);
}

/** Return the canonical HashRouter value without exposing entity identifiers. */
export function currentPageHash(): string {
  if (typeof window === 'undefined') return '';
  return window.location.hash.startsWith('#/') ? `#${currentAppPath()}` : '';
}
