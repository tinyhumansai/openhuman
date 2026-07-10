/**
 * Regression tests for the active-user boot decision that gates #4545.
 *
 * The end-to-end loop the fix prevents (see `resolveActiveUserBootstrap`
 * docblock):
 *   1. User picks Cloud/Remote core → boot succeeds
 *   2. `CoreStateProvider` fetches the remote snapshot → nextIdentity = REMOTE user
 *   3. Boot primes seed from local `active_user.toml` → seedUserId = OLD LOCAL user
 *   4. `seedUserId !== nextIdentity` → `handleIdentityFlip` → `restartApp`
 *   5. Repeat forever
 *
 * The fix short-circuits step 3 whenever `coreMode === 'cloud'` (or the
 * window is a standalone native mascot/notch webview with no Tauri IPC).
 */
import { describe, expect, test, vi } from 'vitest';

import { resolveActiveUserBootstrap, shouldSkipLocalActiveUserRead } from '../bootstrapActiveUser';

describe('shouldSkipLocalActiveUserRead', () => {
  test('cloud mode skips the local read', () => {
    expect(
      shouldSkipLocalActiveUserRead({ isStandaloneNativeWindow: false, coreMode: 'cloud' })
    ).toBe(true);
  });

  test('local mode reads through', () => {
    expect(
      shouldSkipLocalActiveUserRead({ isStandaloneNativeWindow: false, coreMode: 'local' })
    ).toBe(false);
  });

  test('unset mode reads through (fresh install falls back to local file → likely null → prime(null))', () => {
    expect(shouldSkipLocalActiveUserRead({ isStandaloneNativeWindow: false, coreMode: null })).toBe(
      false
    );
  });

  test('standalone native window skips regardless of core mode', () => {
    expect(
      shouldSkipLocalActiveUserRead({ isStandaloneNativeWindow: true, coreMode: 'local' })
    ).toBe(true);
    expect(
      shouldSkipLocalActiveUserRead({ isStandaloneNativeWindow: true, coreMode: 'cloud' })
    ).toBe(true);
  });
});

describe('resolveActiveUserBootstrap', () => {
  test('cloud mode resolves null WITHOUT calling getActiveUserIdFromCore (#4545)', async () => {
    const getActiveUserIdFromCore = vi.fn().mockResolvedValue('stale-local-user');
    const result = await resolveActiveUserBootstrap({
      isStandaloneNativeWindow: false,
      coreMode: 'cloud',
      getActiveUserIdFromCore,
    });
    expect(result).toBeNull();
    expect(getActiveUserIdFromCore).not.toHaveBeenCalled();
  });

  test('local mode delegates to getActiveUserIdFromCore', async () => {
    const getActiveUserIdFromCore = vi.fn().mockResolvedValue('user-A');
    const result = await resolveActiveUserBootstrap({
      isStandaloneNativeWindow: false,
      coreMode: 'local',
      getActiveUserIdFromCore,
    });
    expect(result).toBe('user-A');
    expect(getActiveUserIdFromCore).toHaveBeenCalledTimes(1);
  });

  test('unset mode delegates to getActiveUserIdFromCore (pre-picker cold boot)', async () => {
    const getActiveUserIdFromCore = vi.fn().mockResolvedValue(null);
    const result = await resolveActiveUserBootstrap({
      isStandaloneNativeWindow: false,
      coreMode: null,
      getActiveUserIdFromCore,
    });
    expect(result).toBeNull();
    expect(getActiveUserIdFromCore).toHaveBeenCalledTimes(1);
  });

  test('standalone native window resolves null WITHOUT calling the IPC', async () => {
    const getActiveUserIdFromCore = vi.fn().mockResolvedValue('should-not-be-read');
    const result = await resolveActiveUserBootstrap({
      isStandaloneNativeWindow: true,
      coreMode: 'local',
      getActiveUserIdFromCore,
    });
    expect(result).toBeNull();
    expect(getActiveUserIdFromCore).not.toHaveBeenCalled();
  });
});
