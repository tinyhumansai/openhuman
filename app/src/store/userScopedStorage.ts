/**
 * User-scoped redux-persist storage. Wraps `localStorage` so every key is
 * namespaced by `userId`, e.g. `persist:accounts` → `${userId}:persist:accounts`.
 *
 * This is the durable half of the cross-user leak fix in [#900]: the in-memory
 * Redux reset clears the live store on identity flip, but the localStorage
 * blob has to be partitioned per user so user A's data survives B's session
 * (and rehydrates when A returns) without leaking into B.
 *
 * The active user id is sourced from the standalone `OPENHUMAN_ACTIVE_USER_ID`
 * key, written by `setActiveUserId(...)`. The key is read once at module load
 * so redux-persist's first-paint rehydrate sees the right namespace; later
 * changes call the setter, which updates the in-memory ref and persists the id
 * to localStorage so the *next* cold launch is also seeded.
 *
 * When `activeUserId` is `null` (signed-out), all reads return `null` and all
 * writes are silent no-ops. This is intentional — we never want to write a
 * user-shaped blob to a global key, and we never want to rehydrate a stale
 * blob into a signed-out shell.
 */

const ACTIVE_USER_KEY = 'OPENHUMAN_ACTIVE_USER_ID';

function safeGetActiveUserIdSync(): string | null {
  try {
    return localStorage.getItem(ACTIVE_USER_KEY);
  } catch {
    return null;
  }
}

let activeUserId: string | null = safeGetActiveUserIdSync();

/**
 * Returns the userId currently in scope for persisted reads/writes, or `null`
 * if no user is active yet. Reads through to the latest set value.
 */
export function getActiveUserId(): string | null {
  return activeUserId;
}

/**
 * Update the active user id for redux-persist storage scoping. Pass `null`
 * for sign-out so subsequent persisted writes are dropped on the floor.
 *
 * Persisted to `localStorage[OPENHUMAN_ACTIVE_USER_ID]` so the next cold
 * launch can seed `activeUserId` synchronously before redux-persist
 * rehydrates.
 */
export function setActiveUserId(id: string | null): void {
  activeUserId = id;
  try {
    if (id) {
      localStorage.setItem(ACTIVE_USER_KEY, id);
    } else {
      localStorage.removeItem(ACTIVE_USER_KEY);
    }
  } catch {
    // localStorage may be unavailable (private mode quota); swallowing is
    // fine — the in-memory ref still drives the current session.
  }
}

function namespacedKey(key: string): string | null {
  if (!activeUserId) return null;
  return `${activeUserId}:${key}`;
}

/**
 * `Storage`-shaped object compatible with redux-persist's storage contract.
 * Methods return promises because redux-persist treats storage as async.
 */
export const userScopedStorage = {
  getItem(key: string): Promise<string | null> {
    const ns = namespacedKey(key);
    if (!ns) return Promise.resolve(null);
    try {
      return Promise.resolve(localStorage.getItem(ns));
    } catch {
      return Promise.resolve(null);
    }
  },
  setItem(key: string, value: string): Promise<void> {
    const ns = namespacedKey(key);
    if (!ns) return Promise.resolve();
    try {
      localStorage.setItem(ns, value);
    } catch {
      // ignore quota / unavailable
    }
    return Promise.resolve();
  },
  removeItem(key: string): Promise<void> {
    const ns = namespacedKey(key);
    if (!ns) return Promise.resolve();
    try {
      localStorage.removeItem(ns);
    } catch {
      // ignore
    }
    return Promise.resolve();
  },
};
