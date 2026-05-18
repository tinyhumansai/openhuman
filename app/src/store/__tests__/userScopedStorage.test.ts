/**
 * Tests for `userScopedStorage` — focused on the boot-time prime semantics
 * that gate the cloud-mode reload-survival fix. The single-letter test names
 * mirror the comment block at the top of the source file: each scenario
 * covers one path through `primeActiveUserId(...)` + `setActiveUserId(...)`.
 *
 * Use `vi.resetModules()` between tests because `userScopedStorage` reads
 * `localStorage` once at module load (`safeGetActiveUserIdSync`) and gates
 * subsequent prime calls behind a one-shot `primed` flag — fresh imports
 * exercise the boot path cleanly.
 */
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

const ACTIVE_USER_KEY = 'OPENHUMAN_ACTIVE_USER_ID';

async function importModule() {
  vi.resetModules();
  return import('../userScopedStorage');
}

describe('userScopedStorage', () => {
  beforeEach(() => {
    localStorage.clear();
  });
  afterEach(() => {
    localStorage.clear();
  });

  test('primeActiveUserId(id) writes the seed to localStorage and getActiveUserId returns it', async () => {
    const mod = await importModule();
    mod.primeActiveUserId('user-123');
    expect(mod.getActiveUserId()).toBe('user-123');
    expect(localStorage.getItem(ACTIVE_USER_KEY)).toBe('user-123');
  });

  test('primeActiveUserId(null) preserves existing seed (cloud-mode reload survival)', async () => {
    // Seed a prior value, as if `setActiveUserId(X)` ran in the previous
    // session before `handleIdentityFlip → restartApp`.
    localStorage.setItem(ACTIVE_USER_KEY, 'prior-user');
    const mod = await importModule();

    // Cloud-mode boot can't read `~/.openhuman/active_user.toml` (no local
    // core), so `getActiveUserIdFromCore()` resolves to null. The fix:
    // prime(null) must NOT wipe the seed, otherwise the next snapshot's
    // identity-flip detection re-triggers the loop.
    mod.primeActiveUserId(null);
    expect(mod.getActiveUserId()).toBe('prior-user');
    expect(localStorage.getItem(ACTIVE_USER_KEY)).toBe('prior-user');
  });

  test('primeActiveUserId(null) with no prior seed leaves activeUserId null', async () => {
    const mod = await importModule();
    mod.primeActiveUserId(null);
    expect(mod.getActiveUserId()).toBeNull();
    expect(localStorage.getItem(ACTIVE_USER_KEY)).toBeNull();
  });

  test('primeActiveUserId is one-shot — second call has no effect', async () => {
    const mod = await importModule();
    mod.primeActiveUserId('first');
    mod.primeActiveUserId('second');
    expect(mod.getActiveUserId()).toBe('first');
  });

  test('setActiveUserId(id) writes through to localStorage', async () => {
    const mod = await importModule();
    mod.setActiveUserId('after-login');
    expect(mod.getActiveUserId()).toBe('after-login');
    expect(localStorage.getItem(ACTIVE_USER_KEY)).toBe('after-login');
  });

  test('setActiveUserId(null) clears the seed (sign-out path)', async () => {
    localStorage.setItem(ACTIVE_USER_KEY, 'someone');
    const mod = await importModule();
    mod.setActiveUserId(null);
    expect(mod.getActiveUserId()).toBeNull();
    expect(localStorage.getItem(ACTIVE_USER_KEY)).toBeNull();
  });

  test('setActiveUserId tolerates localStorage failures without throwing', async () => {
    const mod = await importModule();
    const setItemSpy = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    try {
      // Must not throw — the in-memory ref still drives reads.
      expect(() => mod.setActiveUserId('x')).not.toThrow();
      expect(mod.getActiveUserId()).toBe('x');
    } finally {
      setItemSpy.mockRestore();
    }
  });

  test('userScopedStorage redacts auth/session tokens before persisting redux blobs (#1938)', async () => {
    const mod = await importModule();
    mod.primeActiveUserId('user-123');

    const persistedReduxBlob = JSON.stringify({
      sessionToken: 'top-level-token',
      auth: JSON.stringify({
        isAuthenticated: true,
        sessionToken: 'nested-token',
        accessToken: 'nested-access-token',
      }),
      nestedObject: {
        token: 'object-token',
        child: { refreshToken: 'object-refresh-token', safe: 'keep-me' },
      },
      nestedArray: [{ accessToken: 'array-access-token', safe: 'array-safe' }],
      harmless: JSON.stringify({ theme: 'dark' }),
    });

    await mod.userScopedStorage.setItem('persist:coreState', persistedReduxBlob);

    const stored = localStorage.getItem('user-123:persist:coreState');
    expect(stored).not.toBeNull();
    expect(stored).not.toContain('top-level-token');
    expect(stored).not.toContain('nested-token');
    expect(stored).not.toContain('nested-access-token');
    expect(stored).not.toContain('object-token');
    expect(stored).not.toContain('object-refresh-token');
    expect(stored).not.toContain('array-access-token');
    expect(stored).toContain('keep-me');
    expect(stored).toContain('array-safe');
    expect(stored).toContain('dark');
  });
});
