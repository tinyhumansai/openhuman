// Remembers which in-app route started an OAuth connect flow so the `openhuman://oauth/success`
// deep link can return the user there instead of always landing on the connections tab.
//
// The value is consumed only on `oauth/success`. A flow that is canceled, fails before
// completion, or returns `oauth/error` never consumes it, so it must not leak into a later
// unrelated OAuth success. Two guards prevent that: callers clear it on their own failure/error
// paths (clearOAuthReturnRoute), and a freshness TTL bounds the window for a silently abandoned
// flow (e.g. the user closes the consent tab and no deep link ever returns).
const STORAGE_KEY = 'openhuman:oauth:return-route';
const DEFAULT_ROUTE = '/connections';
const MAX_AGE_MS = 5 * 60 * 1000;

interface StoredReturnRoute {
  route: string;
  ts: number;
}

/** Record the hash route that initiated an OAuth connect (e.g. '/rewards'). */
export function setOAuthReturnRoute(route: string): void {
  try {
    const payload: StoredReturnRoute = { route, ts: Date.now() };
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // sessionStorage unavailable (private mode / non-browser host) — fall back to the default.
  }
}

/** Forget any remembered route. Call on OAuth failure/cancel so it can't leak into a later flow. */
export function clearOAuthReturnRoute(): void {
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore — nothing to clear if storage is unavailable.
  }
}

/**
 * Read and clear the remembered OAuth return route. Returns the connections tab unless a valid,
 * in-app, non-stale route was stored by the flow that just succeeded.
 */
export function takeOAuthReturnRoute(): string {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    sessionStorage.removeItem(STORAGE_KEY);
    if (!raw) return DEFAULT_ROUTE;
    const parsed = JSON.parse(raw) as Partial<StoredReturnRoute>;
    const route = typeof parsed.route === 'string' ? parsed.route : null;
    const ts = typeof parsed.ts === 'number' ? parsed.ts : 0;
    if (!route || !route.startsWith('/')) return DEFAULT_ROUTE;
    if (Date.now() - ts > MAX_AGE_MS) return DEFAULT_ROUTE;
    return route;
  } catch {
    return DEFAULT_ROUTE;
  }
}
