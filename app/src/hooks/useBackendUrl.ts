import { useEffect, useState } from 'react';

import { getBackendUrl } from '../services/backendUrl';

/**
 * Resolves the runtime backend URL from the core sidecar (or web fallback)
 * for use inside React components. Returns `null` while the resolution is in
 * flight or if it fails. Components should treat `null` as "URL not yet
 * known" and render a placeholder rather than guessing a hardcoded host.
 *
 * The resolution is delegated to `services/backendUrl#getBackendUrl`, which
 * caches the value for the session — using this hook in many components is
 * cheap (one RPC for the whole app).
 */
export function useBackendUrl(): string | null {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getBackendUrl()
      .then(resolved => {
        if (!cancelled) setUrl(resolved);
      })
      .catch(() => {
        if (!cancelled) setUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return url;
}
