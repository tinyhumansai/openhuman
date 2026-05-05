/**
 * Tests for the `useAppUpdate` hook.
 *
 * Covers the state machine transitions driven by direct calls
 * (check / download / install / apply) and the `app-update:status` /
 * `app-update:progress` events that the Rust side emits.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAppUpdate } from '../useAppUpdate';

// `vi.mock` factories are hoisted above top-level `const` declarations, so
// any state they reference must come from `vi.hoisted` (which is also hoisted).
const hoisted = vi.hoisted(() => {
  return {
    mockCheckAppUpdate: vi.fn(),
    mockApplyAppUpdate: vi.fn(),
    mockDownloadAppUpdate: vi.fn(),
    mockInstallAppUpdate: vi.fn(),
    mockIsTauri: vi.fn(() => true),
    statusListeners: [] as ((event: { payload: string }) => void)[],
    progressListeners: [] as ((event: {
      payload: { chunk: number; total: number | null };
    }) => void)[],
  };
});

const {
  mockCheckAppUpdate,
  mockApplyAppUpdate,
  mockDownloadAppUpdate,
  mockInstallAppUpdate,
  mockIsTauri,
  statusListeners,
  progressListeners,
} = hoisted;

vi.mock('../../utils/tauriCommands', () => ({
  checkAppUpdate: hoisted.mockCheckAppUpdate,
  applyAppUpdate: hoisted.mockApplyAppUpdate,
  downloadAppUpdate: hoisted.mockDownloadAppUpdate,
  installAppUpdate: hoisted.mockInstallAppUpdate,
  isTauri: hoisted.mockIsTauri,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(
    (
      event: string,
      handler:
        | ((event: { payload: string }) => void)
        | ((event: { payload: { chunk: number; total: number | null } }) => void)
    ) => {
      if (event === 'app-update:status') {
        hoisted.statusListeners.push(handler as (event: { payload: string }) => void);
      } else if (event === 'app-update:progress') {
        hoisted.progressListeners.push(
          handler as (event: { payload: { chunk: number; total: number | null } }) => void
        );
      }
      return Promise.resolve(() => {
        const list =
          event === 'app-update:status' ? hoisted.statusListeners : hoisted.progressListeners;
        const index = list.indexOf(
          handler as ((event: { payload: string }) => void) &
            ((event: { payload: { chunk: number; total: number | null } }) => void)
        );
        if (index >= 0) list.splice(index, 1);
      });
    }
  ),
}));

const flush = async () => {
  // Allow the listen() promises inside the hook's effect to resolve.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
};

const emitStatus = async (payload: string) => {
  await act(async () => {
    for (const listener of [...statusListeners]) {
      listener({ payload });
    }
  });
};

const emitProgress = async (chunk: number, total: number | null) => {
  await act(async () => {
    for (const listener of [...progressListeners]) {
      listener({ payload: { chunk, total } });
    }
  });
};

describe('useAppUpdate', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    statusListeners.length = 0;
    progressListeners.length = 0;
    mockIsTauri.mockReturnValue(true);
    mockCheckAppUpdate.mockReset();
    mockApplyAppUpdate.mockReset();
    mockDownloadAppUpdate.mockReset();
    mockInstallAppUpdate.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('check()', () => {
    it('moves to "available" when the updater advertises a new version', async () => {
      mockCheckAppUpdate.mockResolvedValueOnce({
        current_version: '0.50.0',
        available: true,
        available_version: '0.51.0',
        body: 'Bug fixes',
      });

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.check();
      });

      expect(result.current.phase).toBe('available');
      expect(result.current.info?.available_version).toBe('0.51.0');
      expect(result.current.error).toBeNull();
    });

    it('moves to "up_to_date" when no update is available', async () => {
      mockCheckAppUpdate.mockResolvedValueOnce({
        current_version: '0.51.0',
        available: false,
        available_version: null,
        body: null,
      });

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.check();
      });

      expect(result.current.phase).toBe('up_to_date');
    });

    it('moves to "error" with the error message when the check throws', async () => {
      mockCheckAppUpdate.mockRejectedValueOnce(new Error('endpoint unreachable'));

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.check();
      });

      expect(result.current.phase).toBe('error');
      expect(result.current.error).toBe('endpoint unreachable');
    });

    it('no-ops when isTauri() is false', async () => {
      mockIsTauri.mockReturnValue(false);

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      const out = await act(async () => result.current.check());

      expect(out).toBeNull();
      expect(mockCheckAppUpdate).not.toHaveBeenCalled();
      expect(result.current.phase).toBe('idle');
    });
  });

  describe('event listeners', () => {
    it('drives the phase from `app-update:status` events', async () => {
      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await emitStatus('downloading');
      expect(result.current.phase).toBe('downloading');

      await emitStatus('ready_to_install');
      expect(result.current.phase).toBe('ready_to_install');

      await emitStatus('installing');
      expect(result.current.phase).toBe('installing');

      await emitStatus('restarting');
      expect(result.current.phase).toBe('restarting');
    });

    it('accumulates `app-update:progress` chunks and tracks total', async () => {
      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await emitStatus('downloading');
      await emitProgress(1024, 8192);
      await emitProgress(2048, 8192);

      expect(result.current.bytesDownloaded).toBe(3072);
      expect(result.current.totalBytes).toBe(8192);
    });

    it('falls back to "error" with a default message on unknown payloads', async () => {
      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await emitStatus('something-bogus');
      expect(result.current.phase).toBe('error');
      expect(result.current.error).toBe('Update failed. See logs for details.');
    });

    it('removes listeners on unmount', async () => {
      const { unmount } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();
      expect(statusListeners.length).toBe(1);
      expect(progressListeners.length).toBe(1);

      unmount();
      expect(statusListeners.length).toBe(0);
      expect(progressListeners.length).toBe(0);
    });
  });

  describe('download()', () => {
    it('moves to ready_to_install on success and stages bytes for install', async () => {
      mockDownloadAppUpdate.mockImplementationOnce(async () => {
        // Real Rust side emits ready_to_install before resolving.
        await emitStatus('ready_to_install');
        return { ready: true, version: '0.51.0', body: null };
      });

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.download();
      });

      expect(result.current.phase).toBe('ready_to_install');
      expect(result.current.error).toBeNull();
    });

    it('surfaces error when download throws', async () => {
      mockDownloadAppUpdate.mockRejectedValueOnce(new Error('disk full'));

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.download();
      });

      expect(result.current.phase).toBe('error');
      expect(result.current.error).toBe('disk full');
    });
  });

  describe('install()', () => {
    it('falls back to applyAppUpdate when no download has been staged', async () => {
      mockApplyAppUpdate.mockResolvedValueOnce(undefined);

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.install();
      });

      expect(mockApplyAppUpdate).toHaveBeenCalledTimes(1);
      expect(mockInstallAppUpdate).not.toHaveBeenCalled();
    });

    it('uses installAppUpdate when bytes are staged', async () => {
      mockDownloadAppUpdate.mockImplementationOnce(async () => {
        await emitStatus('ready_to_install');
        return { ready: true, version: '0.51.0', body: null };
      });
      mockInstallAppUpdate.mockResolvedValueOnce(undefined);

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.download();
      });
      await act(async () => {
        await result.current.install();
      });

      expect(mockInstallAppUpdate).toHaveBeenCalledTimes(1);
      expect(mockApplyAppUpdate).not.toHaveBeenCalled();
    });

    it('surfaces install errors as phase=error', async () => {
      mockDownloadAppUpdate.mockImplementationOnce(async () => {
        await emitStatus('ready_to_install');
        return { ready: true, version: '0.51.0', body: null };
      });
      mockInstallAppUpdate.mockRejectedValueOnce(new Error('disk full'));

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();
      await act(async () => {
        await result.current.download();
      });
      await act(async () => {
        await result.current.install();
      });

      expect(result.current.phase).toBe('error');
      expect(result.current.error).toBe('disk full');
    });
  });

  describe('apply()', () => {
    it('invokes applyAppUpdate when called manually', async () => {
      mockApplyAppUpdate.mockResolvedValueOnce(undefined);

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.apply();
      });

      expect(mockApplyAppUpdate).toHaveBeenCalledTimes(1);
    });

    it('surfaces apply errors as phase=error', async () => {
      mockApplyAppUpdate.mockRejectedValueOnce(new Error('disk full'));

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.apply();
      });

      expect(result.current.phase).toBe('error');
      expect(result.current.error).toBe('disk full');
    });

    it('no-ops when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.apply();
      });

      expect(mockApplyAppUpdate).not.toHaveBeenCalled();
    });
  });

  describe('install() skip paths', () => {
    it('no-ops when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.install();
      });

      expect(mockInstallAppUpdate).not.toHaveBeenCalled();
      expect(mockApplyAppUpdate).not.toHaveBeenCalled();
    });

    it('surfaces error from the apply fallback when no download is staged', async () => {
      mockApplyAppUpdate.mockRejectedValueOnce(new Error('boom'));

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.install();
      });

      expect(result.current.phase).toBe('error');
      expect(result.current.error).toBe('boom');
    });
  });

  describe('check() guards', () => {
    it('skips when a download / install is already in flight', async () => {
      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await emitStatus('downloading');
      expect(result.current.phase).toBe('downloading');

      const out = await act(async () => result.current.check());
      expect(out).toBeNull();
      expect(mockCheckAppUpdate).not.toHaveBeenCalled();
    });
  });

  describe('download() guards', () => {
    it('does not start a second download while one is in flight', async () => {
      let resolveFirst: ((value: unknown) => void) | null = null;
      mockDownloadAppUpdate.mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveFirst = resolve;
          })
      );

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      // Kick off two concurrent downloads — the second should short-circuit.
      let firstPromise: Promise<void> | undefined;
      let secondPromise: Promise<void> | undefined;
      await act(async () => {
        firstPromise = result.current.download();
        secondPromise = result.current.download();
        await Promise.resolve();
      });

      expect(mockDownloadAppUpdate).toHaveBeenCalledTimes(1);

      // Resolve the first one and let both promises settle so the test's
      // afterEach (vi.useRealTimers) doesn't see leftover pending work.
      await act(async () => {
        resolveFirst?.({ ready: true, version: '0.51.0', body: null });
        await firstPromise;
        await secondPromise;
      });
    });
  });

  describe('reset()', () => {
    it('clears error state and returns to idle from a quiet phase', async () => {
      mockCheckAppUpdate.mockRejectedValueOnce(new Error('boom'));

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.check();
      });
      expect(result.current.phase).toBe('error');

      act(() => result.current.reset());
      expect(result.current.phase).toBe('idle');
      expect(result.current.error).toBeNull();
    });
  });

  describe('auto-check cadence', () => {
    it('runs an initial check after the configured delay', async () => {
      mockCheckAppUpdate.mockResolvedValue({
        current_version: '0.50.0',
        available: false,
        available_version: null,
        body: null,
      });

      renderHook(() =>
        useAppUpdate({ initialCheckDelayMs: 1000, recheckIntervalMs: 0, autoDownload: false })
      );
      await flush();

      expect(mockCheckAppUpdate).not.toHaveBeenCalled();

      await act(async () => {
        vi.advanceTimersByTime(1000);
      });
      await act(async () => {
        await Promise.resolve();
      });

      expect(mockCheckAppUpdate).toHaveBeenCalledTimes(1);
    });

    it('repeats checks at the configured interval', async () => {
      mockCheckAppUpdate.mockResolvedValue({
        current_version: '0.50.0',
        available: false,
        available_version: null,
        body: null,
      });

      renderHook(() =>
        useAppUpdate({ initialCheckDelayMs: 100, recheckIntervalMs: 500, autoDownload: false })
      );
      await flush();

      await act(async () => {
        vi.advanceTimersByTime(100);
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(mockCheckAppUpdate).toHaveBeenCalledTimes(1);

      await act(async () => {
        vi.advanceTimersByTime(500);
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(mockCheckAppUpdate).toHaveBeenCalledTimes(2);

      await act(async () => {
        vi.advanceTimersByTime(500);
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(mockCheckAppUpdate).toHaveBeenCalledTimes(3);
    });

    it('skips auto-check when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);

      renderHook(() =>
        useAppUpdate({ initialCheckDelayMs: 100, recheckIntervalMs: 100, autoDownload: false })
      );
      await flush();

      await act(async () => {
        vi.advanceTimersByTime(1000);
      });

      expect(mockCheckAppUpdate).not.toHaveBeenCalled();
    });
  });

  describe('auto-download', () => {
    it('starts a download automatically when phase becomes available', async () => {
      mockCheckAppUpdate.mockResolvedValueOnce({
        current_version: '0.50.0',
        available: true,
        available_version: '0.51.0',
        body: null,
      });
      mockDownloadAppUpdate.mockImplementationOnce(async () => {
        await emitStatus('ready_to_install');
        return { ready: true, version: '0.51.0', body: null };
      });

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: true }));
      await flush();

      await act(async () => {
        await result.current.check();
      });
      expect(result.current.phase).toBe('available');

      // Auto-download grace timer is 1000ms.
      await act(async () => {
        vi.advanceTimersByTime(1000);
      });
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });

      expect(mockDownloadAppUpdate).toHaveBeenCalledTimes(1);
      expect(result.current.phase).toBe('ready_to_install');
    });

    it('does not auto-download when autoDownload is false', async () => {
      mockCheckAppUpdate.mockResolvedValueOnce({
        current_version: '0.50.0',
        available: true,
        available_version: '0.51.0',
        body: null,
      });

      const { result } = renderHook(() => useAppUpdate({ autoCheck: false, autoDownload: false }));
      await flush();

      await act(async () => {
        await result.current.check();
      });
      expect(result.current.phase).toBe('available');

      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
      await act(async () => {
        await Promise.resolve();
      });

      expect(mockDownloadAppUpdate).not.toHaveBeenCalled();
      expect(result.current.phase).toBe('available');
    });
  });
});
