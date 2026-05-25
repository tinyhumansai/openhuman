import createDebug from 'debug';
import { useEffect, useState } from 'react';

import { getBackendUrl } from '../services/backendUrl';

const log = createDebug('app:backend-probe');

export type BackendProbeStatus = 'probing' | 'reachable' | 'unreachable';

const PROBE_TIMEOUT_MS = 2500;

/**
 * Probes the configured backend URL once on mount and reports whether it is
 * reachable. The "Continue locally" CTA on the Welcome screen is gated on
 * `unreachable` so users only see it when the backend OAuth flow can't be
 * completed (per issue #2037 AC).
 *
 * We treat any successful HTTP response (incl. 4xx) as reachable — the goal is
 * to confirm the host is online, not that a specific route exists.
 */
export function useBackendReachable(): BackendProbeStatus {
  const [status, setStatus] = useState<BackendProbeStatus>('probing');

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);

    void (async () => {
      try {
        const base = await getBackendUrl();
        log('[probe] fetching %s/health', base);
        const response = await fetch(`${base}/health`, {
          method: 'GET',
          signal: controller.signal,
          cache: 'no-store',
        });
        if (cancelled) return;
        // Any HTTP response (even 404) means the host answered — it's online.
        log('[probe] response status=%d → reachable', response.status);
        setStatus('reachable');
      } catch (err) {
        if (cancelled) return;
        log('[probe] failed → unreachable: %o', err);
        setStatus('unreachable');
      } finally {
        window.clearTimeout(timeoutId);
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
      window.clearTimeout(timeoutId);
    };
  }, []);

  return status;
}
