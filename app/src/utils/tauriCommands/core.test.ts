import { invoke, isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

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
    test('no-ops when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      const debug = vi.spyOn(console, 'debug').mockImplementation(() => {});

      await restartApp();

      expect(mockInvoke).not.toHaveBeenCalled();
      expect(debug).toHaveBeenCalledWith(
        expect.stringContaining('restartApp: skipped — not running in Tauri')
      );
      debug.mockRestore();
    });

    test('invokes restart_app when in Tauri', async () => {
      await restartApp();
      expect(mockInvoke).toHaveBeenCalledWith('restart_app');
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
