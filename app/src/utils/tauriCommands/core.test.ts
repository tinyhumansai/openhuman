import { invoke, isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/core', () => {
  const mockIsTauri = isTauri as Mock;
  const mockInvoke = invoke as Mock;
  let restartApp: typeof import('./core').restartApp;
  let scheduleCefProfilePurge: typeof import('./core').scheduleCefProfilePurge;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('./core')>('./core');
    restartApp = actual.restartApp;
    scheduleCefProfilePurge = actual.scheduleCefProfilePurge;
  });

  describe('restartApp', () => {
    // window.location.reload is non-configurable on jsdom's default location;
    // swap in a mocked location object for the dev-mode tests and restore after.
    let originalLocation: Location;
    let reloadSpy: Mock;

    beforeEach(() => {
      originalLocation = window.location;
      reloadSpy = vi.fn();
      Object.defineProperty(window, 'location', {
        value: { ...originalLocation, reload: reloadSpy },
        configurable: true,
        writable: true,
      });
    });

    afterEach(() => {
      Object.defineProperty(window, 'location', {
        value: originalLocation,
        configurable: true,
        writable: true,
      });
      vi.unstubAllEnvs();
    });

    test('no-ops when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      const debug = vi.spyOn(console, 'debug').mockImplementation(() => {});

      await restartApp();

      expect(mockInvoke).not.toHaveBeenCalled();
      expect(reloadSpy).not.toHaveBeenCalled();
      expect(debug).toHaveBeenCalledWith(
        expect.stringContaining('restartApp: skipped — not running in Tauri')
      );
      debug.mockRestore();
    });

    test('reloads webview in dev mode (#1068 — avoid orphaning vite)', async () => {
      // setup.ts seeds DEV=true globally; the binding imported above already
      // captured that value, so we just need to invoke the dev-mode branch.
      const debug = vi.spyOn(console, 'debug').mockImplementation(() => {});

      await restartApp();

      expect(reloadSpy).toHaveBeenCalledTimes(1);
      expect(mockInvoke).not.toHaveBeenCalled();
      expect(debug).toHaveBeenCalledWith(
        expect.stringContaining('restartApp: dev mode → window.location.reload()')
      );
      debug.mockRestore();
    });

    test('invokes restart_app in production mode (IS_DEV=false)', async () => {
      // setup.ts globally mocks ../config with IS_DEV: true. Override with
      // doMock + resetModules so a fresh import of ./core sees IS_DEV=false
      // and runs the production branch (#1068 dev workaround should be inert).
      vi.doMock('../config', () => ({
        IS_DEV: false,
        // Re-export anything else core.ts might end up using; today just IS_DEV.
      }));
      vi.resetModules();
      const prodCore = await import('./core');

      await prodCore.restartApp();

      expect(mockInvoke).toHaveBeenCalledWith('restart_app');
      expect(reloadSpy).not.toHaveBeenCalled();

      vi.doUnmock('../config');
    });
  });

  describe('scheduleCefProfilePurge', () => {
    test('returns null and does not invoke when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      const debug = vi.spyOn(console, 'debug').mockImplementation(() => {});

      const out = await scheduleCefProfilePurge('x');

      expect(out).toBeNull();
      expect(mockInvoke).not.toHaveBeenCalled();
      expect(debug).toHaveBeenCalledWith(
        expect.stringContaining('scheduleCefProfilePurge: skipped — not running in Tauri')
      );
      debug.mockRestore();
    });

    test('invoke with userId null when argument is undefined', async () => {
      mockInvoke.mockResolvedValueOnce('/path');

      const out = await scheduleCefProfilePurge();

      expect(mockInvoke).toHaveBeenCalledWith('schedule_cef_profile_purge', { userId: null });
      expect(out).toBe('/path');
    });

    test('invoke with userId null when argument is null', async () => {
      mockInvoke.mockResolvedValueOnce('/path');

      const out = await scheduleCefProfilePurge(null);

      expect(mockInvoke).toHaveBeenCalledWith('schedule_cef_profile_purge', { userId: null });
      expect(out).toBe('/path');
    });

    test('invoke with userId string when provided', async () => {
      mockInvoke.mockResolvedValueOnce('/other');

      const out = await scheduleCefProfilePurge('user-9');

      expect(mockInvoke).toHaveBeenCalledWith('schedule_cef_profile_purge', { userId: 'user-9' });
      expect(out).toBe('/other');
    });
  });
});
