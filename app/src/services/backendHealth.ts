import { getBackendUrl } from './backendUrl';

export const BACKEND_HEALTH_TIMEOUT_MS = 6_000;

export type BackendHealthFailureReason =
  | 'timeout' // AbortError after BACKEND_HEALTH_TIMEOUT_MS
  | 'network' // fetch rejected before any HTTP response (DNS, CORS, offline, TLS)
  | 'http-5xx' // upstream/edge returned a 5xx (e.g. Cloudflare 504 gateway timeout)
  | 'resolve-failure'; // could not resolve the backend URL at all

export type BackendHealthResult =
  | { healthy: true; status: number; latencyMs: number; backendUrl: string }
  | { healthy: false; reason: BackendHealthFailureReason; status?: number; latencyMs: number };

interface CheckOptions {
  timeoutMs?: number;
  fetchImpl?: typeof fetch;
}

/**
 * Probes the backend `/health` endpoint.
 *
 * `GET /health` on the OpenHuman backend returns 200 `{"status":"ok"}`. We
 * treat any 2xx/3xx/4xx as "the backend is reachable at all" — the goal of
 * this probe is specifically to catch full edge/origin failures (Cloudflare
 * 5xx, DNS, offline) so we can surface them on the Welcome screen instead of
 * silently sending the user into a system browser that lands on an error page.
 *
 * **Never throws.** All network, timeout, and URL-resolution failures are
 * caught internally and surfaced as `{ healthy: false, reason: … }` results,
 * so callers do not need to wrap this in try/catch. The healthy variant also
 * returns the resolved `backendUrl` so callers can reuse it without a second
 * `getBackendUrl()` round-trip.
 */
export async function checkBackendHealthy(
  options: CheckOptions = {}
): Promise<BackendHealthResult> {
  const { timeoutMs = BACKEND_HEALTH_TIMEOUT_MS, fetchImpl = fetch } = options;
  const start = Date.now();

  let backendUrl: string;
  try {
    backendUrl = await getBackendUrl();
  } catch (err) {
    console.debug('[backend-health] could not resolve backend URL', err);
    return { healthy: false, reason: 'resolve-failure', latencyMs: 0 };
  }

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const response = await fetchImpl(`${backendUrl}/health`, {
      method: 'GET',
      cache: 'no-store',
      credentials: 'omit',
      signal: controller.signal,
    });
    const latencyMs = Date.now() - start;

    if (response.status >= 500 && response.status < 600) {
      console.debug(`[backend-health] unhealthy: HTTP ${response.status} in ${latencyMs}ms`);
      return { healthy: false, reason: 'http-5xx', status: response.status, latencyMs };
    }

    return { healthy: true, status: response.status, latencyMs, backendUrl };
  } catch (err) {
    const latencyMs = Date.now() - start;
    if (err instanceof DOMException && err.name === 'AbortError') {
      console.debug(`[backend-health] timeout after ${latencyMs}ms`);
      return { healthy: false, reason: 'timeout', latencyMs };
    }
    console.debug(`[backend-health] network error after ${latencyMs}ms`, err);
    return { healthy: false, reason: 'network', latencyMs };
  } finally {
    clearTimeout(timer);
  }
}
