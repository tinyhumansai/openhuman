import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  incrementConnectionAttempts,
  resetConnectionAttempts,
  setAutoStartEnabled,
  setDaemonStatus,
  setIsRecovering,
} from '../../features/daemon/store';
import { isTauri } from '../../utils/tauriCommands';

const mockStartDaemon = vi.fn();
const mockRestartDaemon = vi.fn();

vi.mock('../useDaemonHealth', () => ({
  useDaemonHealth: () => ({ startDaemon: mockStartDaemon, restartDaemon: mockRestartDaemon }),
}));

const setVisibility = (value: 'visible' | 'hidden'): void => {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => value });
  document.dispatchEvent(new Event('visibilitychange'));
};

const freshUser = (() => {
  let n = 0;
  return (prefix: string): string => `${prefix}-${++n}-${Date.now()}`;
})();

const resetUser = (uid: string): void => {
  resetConnectionAttempts(uid);
  setAutoStartEnabled(uid, false);
  setIsRecovering(uid, false);
  setDaemonStatus(uid, 'disconnected');
};

describe('useDaemonLifecycle', () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
    mockStartDaemon.mockReset();
    mockRestartDaemon.mockReset();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('exponential backoff bounds', () => {
    it('nextRetryDelay doubles from 1s as attempts increase', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('backoff');
      resetUser(uid);

      const { result } = renderHook(() => useDaemonLifecycle(uid));

      // attempts=0 → next attempt is #1 → 1000 * 2^0 = 1000
      expect(result.current.nextRetryDelay).toBe(1000);

      const expected = [2000, 4000, 8000, 16000];
      for (const delay of expected) {
        act(() => incrementConnectionAttempts(uid));
        expect(result.current.nextRetryDelay).toBe(delay);
      }
    });

    it('every computed delay stays within [BASE, MAX_RETRY_DELAY_MS] bounds', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('bounds');
      resetUser(uid);

      const { result } = renderHook(() => useDaemonLifecycle(uid));
      const observed: number[] = [];

      observed.push(result.current.nextRetryDelay ?? -1);
      for (let i = 0; i < result.current.MAX_RECONNECTION_ATTEMPTS - 1; i++) {
        act(() => incrementConnectionAttempts(uid));
        if (result.current.nextRetryDelay != null) {
          observed.push(result.current.nextRetryDelay);
        }
      }

      for (const delay of observed) {
        expect(delay).toBeGreaterThanOrEqual(1000);
        expect(delay).toBeLessThanOrEqual(30000);
      }
      // Monotonically non-decreasing (doubling, eventually capped).
      for (let i = 1; i < observed.length; i++) {
        expect(observed[i]).toBeGreaterThanOrEqual(observed[i - 1]);
      }
    });
  });

  describe('maxAttemptsReached', () => {
    it('is false below MAX and flips true at/above MAX, with nextRetryDelay null', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('max');
      resetUser(uid);

      const { result } = renderHook(() => useDaemonLifecycle(uid));

      expect(result.current.maxAttemptsReached).toBe(false);
      const max = result.current.MAX_RECONNECTION_ATTEMPTS;

      act(() => {
        for (let i = 0; i < max - 1; i++) incrementConnectionAttempts(uid);
      });
      expect(result.current.maxAttemptsReached).toBe(false);
      expect(result.current.nextRetryDelay).not.toBeNull();

      act(() => incrementConnectionAttempts(uid));
      expect(result.current.connectionAttempts).toBe(max);
      expect(result.current.maxAttemptsReached).toBe(true);
      expect(result.current.nextRetryDelay).toBeNull();
    });

    it('resetRetries clears attempts and re-enables retries', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('reset');
      resetUser(uid);

      const { result } = renderHook(() => useDaemonLifecycle(uid));
      act(() => {
        for (let i = 0; i < result.current.MAX_RECONNECTION_ATTEMPTS; i++) {
          incrementConnectionAttempts(uid);
        }
      });
      expect(result.current.maxAttemptsReached).toBe(true);

      act(() => result.current.resetRetries());

      expect(result.current.connectionAttempts).toBe(0);
      expect(result.current.maxAttemptsReached).toBe(false);
      expect(result.current.nextRetryDelay).toBe(1000);
    });
  });

  describe('background / foreground pause-resume', () => {
    it('does not invoke startDaemon while hidden, resumes auto-start on visible', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('vis');
      resetUser(uid);
      // Enable auto-start before mount so the visibility listener captures the
      // "disconnected + autoStart + !recovering" branch on the very first render.
      setAutoStartEnabled(uid, true);
      mockStartDaemon.mockResolvedValue({ result: { state: 'Running' }, logs: [] });
      setVisibility('visible');

      renderHook(() => useDaemonLifecycle(uid));

      // Going hidden must not schedule any auto-start work.
      setVisibility('hidden');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(mockStartDaemon).not.toHaveBeenCalled();

      // Returning to foreground schedules a delayed auto-start (1000ms inside the handler).
      // We stop asserting before the 3000ms initial auto-start timer window so this test
      // isolates the resume branch rather than the mount branch.
      setVisibility('visible');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
      expect(mockStartDaemon).toHaveBeenCalledTimes(1);
    });

    it('visibility handler is a no-op when auto-start is disabled', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('vis-off');
      resetUser(uid);
      setAutoStartEnabled(uid, false);
      setVisibility('visible');

      renderHook(() => useDaemonLifecycle(uid));

      // No initial auto-start scheduled; no startDaemon call.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000);
      });
      expect(mockStartDaemon).not.toHaveBeenCalled();

      setVisibility('hidden');
      setVisibility('visible');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(mockStartDaemon).not.toHaveBeenCalled();
    });

    it('skips resume when status is already running', async () => {
      const { useDaemonLifecycle } = await import('../useDaemonLifecycle');
      const uid = freshUser('vis-running');
      resetUser(uid);
      setAutoStartEnabled(uid, true);
      setDaemonStatus(uid, 'running');
      setVisibility('visible');

      renderHook(() => useDaemonLifecycle(uid));

      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000);
      });
      // Initial auto-start runs but attemptAutoStart bails because status !== 'disconnected'.
      expect(mockStartDaemon).not.toHaveBeenCalled();

      setVisibility('hidden');
      setVisibility('visible');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(mockStartDaemon).not.toHaveBeenCalled();
    });
  });
});
