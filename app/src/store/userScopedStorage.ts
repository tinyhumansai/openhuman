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

// Gate redux-persist's rehydrate on the boot prime from main.tsx
// (which reads the authoritative id from `~/.openhuman/active_user.toml`
// via the Rust core). The localStorage value used at module load is
// bound to the per-user CEF profile dir and goes stale across
// restart-driven user flips, so storage reads must wait for the
// asynchronous prime before resolving the namespace. (#900)
let activeUserIdResolve!: () => void;
const activeUserIdReady = new Promise<void>(resolve => {
  activeUserIdResolve = resolve;
});
let primed = false;

/**
 * Mark `userScopedStorage` as primed with the boot-time active user id.
 *
 * Called once by `main.tsx` after `getActiveUserIdFromCore()` returns.
 * Pass `null` for "no user logged in yet" — storage reads/writes then
 * fall through as no-ops until a real id is supplied later via
 * `setActiveUserId`.
 *
 * Safe to call before `setActiveUserId` for an initial seed; subsequent
 * `primeActiveUserId(...)` calls have no effect (the gate is one-shot).
 */
export function primeActiveUserId(id: string | null): void {
  if (primed) return;
  primed = true;
  activeUserId = id;
  try {
    if (id) {
      localStorage.setItem(ACTIVE_USER_KEY, id);
    } else {
      localStorage.removeItem(ACTIVE_USER_KEY);
    }
  } catch {
    // localStorage may be unavailable; in-memory ref still drives reads
  }
  activeUserIdResolve();
}

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
  const previous = activeUserId;
  activeUserId = id;
  try {
    if (id) {
      localStorage.setItem(ACTIVE_USER_KEY, id);
      if (!previous) {
        migrateLegacyPersistKeys(id);
      }
    } else {
      localStorage.removeItem(ACTIVE_USER_KEY);
    }
  } catch {
    // localStorage may be unavailable (private mode quota); swallowing is
    // fine — the in-memory ref still drives the current session.
  }
}

/**
 * One-shot migration for users upgrading from the pre-#900 build, where
 * persist blobs lived at unscoped keys (`persist:accounts`, etc.). On the
 * first identity assignment after launch, if any legacy key exists and the
 * corresponding user-scoped key is empty, copy legacy → `${id}:<key>` and
 * drop the legacy entry. This lets the FIRST user to log in on the upgraded
 * build keep their UI shimmer; later users see initial state and rehydrate
 * from backend as usual.
 */
function migrateLegacyPersistKeys(id: string): void {
  const LEGACY_PREFIXES = ['persist:'];
  try {
    const legacyKeys: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key) continue;
      if (LEGACY_PREFIXES.some(p => key.startsWith(p))) {
        legacyKeys.push(key);
      }
    }
    for (const key of legacyKeys) {
      const scoped = `${id}:${key}`;
      if (localStorage.getItem(scoped) !== null) continue; // already migrated
      const value = localStorage.getItem(key);
      if (value === null) continue;
      localStorage.setItem(scoped, value);
      localStorage.removeItem(key);
    }
  } catch {
    // best-effort; ignore quota / unavailable
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
  async getItem(key: string): Promise<string | null> {
    await activeUserIdReady;
    const ns = namespacedKey(key);
    if (!ns) return null;
    try {
      return localStorage.getItem(ns);
    } catch {
      return null;
    }
  },
  async setItem(key: string, value: string): Promise<void> {
    await activeUserIdReady;
    const ns = namespacedKey(key);
    if (!ns) return;
    try {
      localStorage.setItem(ns, value);
    } catch {
      // ignore quota / unavailable
    }
  },
  async removeItem(key: string): Promise<void> {
    await activeUserIdReady;
    const ns = namespacedKey(key);
    if (!ns) return;
    try {
      localStorage.removeItem(ns);
    } catch {
      // ignore
    }
  },
};
