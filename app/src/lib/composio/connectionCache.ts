/**
 * Client-side cache for the Composio connection ("activation") state.
 *
 * Composio connections are backend-managed: `useComposioIntegrations` fetches
 * them fresh on every mount and reconciles via a 5s poll. That makes the
 * source of truth durable, but it also means a cold app restart shows an empty
 * loading skeleton until the first round-trip lands — even though the user
 * already authorised those toolkits. To an activation flow that is supposed to
 * "survive an app restart" (issue #4273, AC1) that reads as a regression: the
 * Skills page momentarily looks disconnected.
 *
 * This module is the durable half of that flow. It mirrors the sibling
 * `catalogCache.ts` (synchronous `localStorage` + in-memory mirror) so the hook
 * can seed its initial state instantly on mount and render the last-known
 * connected toolkits with no flash, then reconcile against the live backend
 * fetch. It is intentionally NOT the source of truth — a stale cache is always
 * corrected by the very next fetch/poll a few seconds later.
 *
 * The cache key is namespaced by the active user id (the same id
 * `userScopedStorage` uses) so user A's connections never bleed into user B's
 * session after an identity flip (#900).
 *
 * `clearConnectionCache()` drops both tiers — call it when the Composio client
 * identity changes (backend ↔ direct mode, BYO API key) or whenever the live
 * state is known to be "no connections", exactly like the existing
 * `composio:config-changed` refresh path does for the catalog.
 */
import type { ComposioConnection, ComposioToolkitCatalogEntry } from './types';

const CACHE_KEY_SUFFIX = 'composio:connections:v1';
/**
 * Bound how stale a seeded snapshot may be. The value only gates the *initial
 * paint* — the hook always issues a live fetch on mount regardless — so this is
 * a safety net against rendering very old state if the backend is unreachable,
 * not a freshness contract.
 */
const TTL_MS = 24 * 60 * 60 * 1000;

const ACTIVE_USER_KEY = 'OPENHUMAN_ACTIVE_USER_ID';

export interface CachedConnectionState {
  fetchedAt: number;
  connections: ComposioConnection[];
  toolkits: string[];
  catalog: ComposioToolkitCatalogEntry[];
}

/** In-memory mirror so repeated reads on the hot path skip a `JSON.parse`. */
let memory: CachedConnectionState | null = null;
/** The user id `memory` belongs to, so a mid-session flip re-reads from disk. */
let memoryUserId: string | null = null;

/**
 * The active user id, or `null` when no user is identified yet (pre-login /
 * mid identity-flip). When `null` the cache is fully inert — reads return
 * `null` and writes are no-ops — mirroring `userScopedStorage`'s signed-out
 * behaviour so two unidentified sessions can never share a blob and leak one
 * user's Composio connections into another's shell.
 */
function activeUserId(): string | null {
  try {
    const id = window.localStorage.getItem(ACTIVE_USER_KEY);
    return id && id.length > 0 ? id : null;
  } catch {
    return null;
  }
}

function cacheKey(userId: string): string {
  // User id FIRST so the key matches userScopedStorage's `${userId}:...` shape.
  // clearAllAppData's purge removes every `${userId}:` key, so this namespacing
  // ensures "clear my data" also drops the connection cache instead of leaving
  // it to re-seed stale connected toolkits on the next sign-in (PR #4288).
  return `${userId}:${CACHE_KEY_SUFFIX}`;
}

/** A cached snapshot is usable only while still inside the TTL window. */
function isFresh(entry: CachedConnectionState): boolean {
  return Date.now() - entry.fetchedAt < TTL_MS;
}

function isValid(parsed: unknown): parsed is CachedConnectionState {
  if (!parsed || typeof parsed !== 'object') return false;
  const entry = parsed as Partial<CachedConnectionState>;
  return (
    typeof entry.fetchedAt === 'number' &&
    Array.isArray(entry.connections) &&
    Array.isArray(entry.toolkits) &&
    Array.isArray(entry.catalog)
  );
}

/**
 * Read the last-known connection snapshot for the active user, or `null` when
 * absent / malformed / expired. Cheap on repeat calls thanks to the in-memory
 * mirror; the mirror is dropped automatically when the active user changes.
 */
export function readConnectionCache(): CachedConnectionState | null {
  const userId = activeUserId();
  // No identified user → cache is inert so two unidentified sessions can never
  // share a blob and leak one user's connections into another's shell.
  if (userId === null) return null;
  if (memory && memoryUserId === userId) {
    return isFresh(memory) ? memory : null;
  }
  try {
    const raw = window.localStorage.getItem(cacheKey(userId));
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!isValid(parsed)) return null;
    // Never promote an expired snapshot into the in-memory mirror: a later
    // partial write (e.g. a connections-only poll after a failed toolkit fetch)
    // would otherwise merge against — and revive — stale toolkit/catalog data
    // the TTL was meant to suppress (PR #4288).
    if (!isFresh(parsed)) return null;
    memory = parsed;
    memoryUserId = userId;
    return parsed;
  } catch {
    return null;
  }
}

/**
 * Persist (merge) the latest known connection state. Any field omitted from
 * `patch` keeps its previously cached value, so the 5s poll can refresh just
 * `connections` without clobbering the toolkit allowlist / catalog. `fetchedAt`
 * is bumped on every write so the freshest poll wins the next cold paint.
 */
export function writeConnectionCache(patch: {
  connections?: ComposioConnection[];
  toolkits?: string[];
  catalog?: ComposioToolkitCatalogEntry[];
}): void {
  const userId = activeUserId();
  // Inert until a user is identified — mirrors userScopedStorage's signed-out
  // no-op so we never write a user-shaped blob to a shared key.
  if (userId === null) return;
  const base =
    memory && memoryUserId === userId && isFresh(memory)
      ? memory
      : { fetchedAt: 0, connections: [], toolkits: [], catalog: [] };
  const entry: CachedConnectionState = {
    fetchedAt: Date.now(),
    connections: patch.connections ?? base.connections,
    toolkits: patch.toolkits ?? base.toolkits,
    catalog: patch.catalog ?? base.catalog,
  };
  memory = entry;
  memoryUserId = userId;
  try {
    window.localStorage.setItem(cacheKey(userId), JSON.stringify(entry));
  } catch {
    // Private-mode / quota errors are non-fatal — the in-memory mirror still
    // serves this session.
  }
}

/** Drop both cache tiers for the active user so the next read re-fetches. */
export function clearConnectionCache(): void {
  const userId = activeUserId();
  // Always drop the in-memory mirror; only the keyed localStorage entry needs a
  // user id (nothing to remove when none is set).
  memory = null;
  memoryUserId = null;
  if (userId === null) return;
  try {
    window.localStorage.removeItem(cacheKey(userId));
  } catch {
    // ignore
  }
}
