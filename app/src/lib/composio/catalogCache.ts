/**
 * Client-side cache for the Composio toolkit catalog.
 *
 * Mirrors the backend's 24h cache (see the workspace
 * `COMPOSIO_DYNAMIC_CATALOG_PLAN.md`). The catalog only changes when
 * Composio adds/removes toolkits, so re-fetching it on every Skills-page
 * mount is wasteful. We layer two guards in front of `listToolkits()`:
 *
 *   1. **In-flight dedupe** — N components mounting at once share a single
 *      RPC instead of firing one each. Race-free because the JS event loop
 *      never interleaves the synchronous check-then-assign below.
 *   2. **localStorage TTL (24h)** — survives reloads and serves instantly
 *      on a warm cache; falls back to a live fetch when stale/absent.
 *
 * `invalidateToolkitCatalogCache()` clears both tiers — call it when the
 * Composio client identity changes (backend ↔ direct mode, BYO API key),
 * exactly like the existing `composio:config-changed` refresh path.
 */
import { listToolkits } from './composioApi';
import type { ComposioToolkitsResponse } from './types';

const CACHE_KEY = 'composio:catalog:v1';
const TTL_MS = 24 * 60 * 60 * 1000;

interface CachedCatalog {
  fetchedAt: number;
  response: ComposioToolkitsResponse;
}

/** Module-level in-flight promise so concurrent callers join one fetch. */
let inflight: Promise<ComposioToolkitsResponse> | null = null;
/** In-memory mirror so we avoid a JSON.parse on the hot path. */
let memory: CachedCatalog | null = null;
/**
 * Bumped on every invalidation. A fetch captures the generation at start and
 * refuses to write its result if the generation has since changed — so a
 * response that was already in flight when the Composio client identity
 * switched (mode toggle / BYO key → `composio:config-changed`) can't poison
 * the cache with the *previous* tenant's catalog and have it served as fresh
 * for 24h.
 */
let generation = 0;

function readPersisted(): CachedCatalog | null {
  if (memory) return memory;
  try {
    const raw = window.localStorage.getItem(CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as CachedCatalog;
    if (
      !parsed ||
      typeof parsed.fetchedAt !== 'number' ||
      !parsed.response ||
      !Array.isArray(parsed.response.toolkits)
    ) {
      return null;
    }
    memory = parsed;
    return parsed;
  } catch {
    return null;
  }
}

function writePersisted(response: ComposioToolkitsResponse): void {
  const entry: CachedCatalog = { fetchedAt: Date.now(), response };
  memory = entry;
  try {
    window.localStorage.setItem(CACHE_KEY, JSON.stringify(entry));
  } catch {
    // Private-mode / quota errors are non-fatal — the in-memory mirror
    // still serves this session.
  }
}

function isFresh(entry: CachedCatalog | null): boolean {
  return entry !== null && Date.now() - entry.fetchedAt < TTL_MS;
}

/**
 * Resolve the toolkit catalog, preferring a fresh client cache.
 *
 * - Fresh cache (< 24h)            → returned immediately, no RPC.
 * - Stale-but-present + fetch ok   → fresh value, cache refreshed.
 * - Stale-but-present + fetch fail → stale value served (graceful degrade).
 * - Cold + fetch fail              → error propagates to the caller.
 */
export async function getToolkitCatalog(options?: {
  /**
   * Forwarded to `listToolkits` on a cold/stale fetch. The Connections-page
   * hook passes `COMPOSIO_FETCH_TIMEOUT_MS` to bound its loading skeleton;
   * omit it to inherit the global RPC default.
   */
  timeoutMs?: number;
}): Promise<ComposioToolkitsResponse> {
  const cached = readPersisted();
  if (cached && isFresh(cached)) return cached.response;

  if (inflight) return inflight;
  // Snapshot the generation so a mid-flight invalidation makes this fetch's
  // result non-authoritative (see `generation`).
  const startGeneration = generation;
  const fetchPromise = listToolkits(options)
    .then(response => {
      // Only cache the response if no invalidation happened while it was in
      // flight; otherwise it belongs to a stale tenant. Still return it to
      // this caller — just don't poison the shared cache for future reads.
      if (generation === startGeneration) {
        writePersisted(response);
      } else {
        console.debug('[composio-cache] discarding catalog response invalidated mid-flight');
      }
      return response;
    })
    .catch(err => {
      // On failure, fall back to a stale cache if we have one rather than
      // forcing the UI into an error state for a list that rarely changes.
      // Skip the fallback if we were invalidated mid-flight — the cached
      // value belongs to the previous tenant.
      if (cached && generation === startGeneration) {
        console.warn(
          '[composio-cache] catalog fetch failed; serving stale cache:',
          err instanceof Error ? err.message : String(err)
        );
        return cached.response;
      }
      throw err;
    })
    .finally(() => {
      // Only clear the slot if it's still ours — an invalidation may have
      // reset `inflight` to null and a newer fetch taken its place.
      if (inflight === fetchPromise) {
        inflight = null;
      }
    });
  inflight = fetchPromise;
  return fetchPromise;
}

/** Drop both cache tiers so the next read re-fetches. */
export function invalidateToolkitCatalogCache(): void {
  generation += 1;
  memory = null;
  inflight = null;
  try {
    window.localStorage.removeItem(CACHE_KEY);
  } catch {
    // ignore
  }
}
