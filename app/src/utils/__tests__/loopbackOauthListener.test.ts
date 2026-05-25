import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { startLoopbackOauthListener } from '../loopbackOauthListener';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

type TauriInternalsHolder = { __TAURI_INTERNALS__?: { invoke: unknown } };

const mockInvoke = invoke as Mock;
const mockListen = listen as Mock;

beforeEach(() => {
  vi.clearAllMocks();
  // Satisfy the isTauri() bootstrap-gap check in utils/tauriCommands/common.ts.
  const holder = window as unknown as TauriInternalsHolder;
  holder.__TAURI_INTERNALS__ = { invoke: () => undefined };
});

describe('startLoopbackOauthListener', () => {
  test('returns null when shell bind fails (fallback to deep link)', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('bind 127.0.0.1:53824 failed: Address in use'));

    const handle = await startLoopbackOauthListener();

    expect(handle).toBeNull();
    expect(mockInvoke).toHaveBeenCalledWith('start_loopback_oauth_listener', {
      port: 53824,
      timeoutSecs: 300,
    });
  });

  test('returns handle with redirect uri and state on success', async () => {
    mockInvoke.mockResolvedValueOnce({
      redirectUri: 'http://127.0.0.1:53824/auth',
      state: 'deadbeef',
    });
    mockListen.mockResolvedValue(() => {});

    const handle = await startLoopbackOauthListener();

    expect(handle).not.toBeNull();
    expect(handle!.state).toBe('deadbeef');
    expect(handle!.redirectUri).toBe('http://127.0.0.1:53824/auth?state=deadbeef');
  });

  test('awaitCallback resolves with URL when shell emits callback event', async () => {
    mockInvoke.mockResolvedValueOnce({
      redirectUri: 'http://127.0.0.1:53824/auth',
      state: 'state-1',
    });
    let registered: ((event: { payload: { url: string } }) => void) | null = null;
    mockListen.mockImplementation((_event, handler) => {
      registered = handler;
      return Promise.resolve(() => {});
    });

    const handle = await startLoopbackOauthListener();
    const callbackPromise = handle!.awaitCallback();
    // Wait a microtask for listen() to register.
    await Promise.resolve();
    registered!({ payload: { url: 'http://127.0.0.1:53824/auth?token=jwt&state=state-1' } });

    await expect(callbackPromise).resolves.toBe(
      'http://127.0.0.1:53824/auth?token=jwt&state=state-1'
    );
  });

  test('cancel calls stop_loopback_oauth_listener', async () => {
    mockInvoke
      .mockResolvedValueOnce({ redirectUri: 'http://127.0.0.1:53824/auth', state: 's' })
      .mockResolvedValueOnce(undefined);
    mockListen.mockResolvedValue(() => {});

    const handle = await startLoopbackOauthListener();
    await handle!.cancel();

    expect(mockInvoke).toHaveBeenNthCalledWith(2, 'stop_loopback_oauth_listener');
  });

  test('cancel swallows stop_loopback_oauth_listener failure', async () => {
    mockInvoke
      .mockResolvedValueOnce({ redirectUri: 'http://127.0.0.1:53824/auth', state: 's' })
      .mockRejectedValueOnce(new Error('already stopped'));
    mockListen.mockResolvedValue(() => {});
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      const handle = await startLoopbackOauthListener();
      await expect(handle!.cancel()).resolves.toBeUndefined();
      expect(warn).toHaveBeenCalledWith('[loopback-oauth] stop failed', expect.any(Error));
    } finally {
      warn.mockRestore();
    }
  });

  test('awaitCallback rejects when listen() rejects', async () => {
    mockInvoke.mockResolvedValueOnce({ redirectUri: 'http://127.0.0.1:53824/auth', state: 's' });
    mockListen.mockRejectedValueOnce(new Error('listen failed'));

    const handle = await startLoopbackOauthListener();
    await expect(handle!.awaitCallback()).rejects.toThrow('listen failed');
  });

  test('awaitCallback rejects on timeout and stops the listener', async () => {
    vi.useFakeTimers();
    try {
      mockInvoke
        .mockResolvedValueOnce({ redirectUri: 'http://127.0.0.1:53824/auth', state: 's' })
        .mockResolvedValueOnce(undefined);
      const unlisten = vi.fn();
      mockListen.mockResolvedValue(unlisten);

      const handle = await startLoopbackOauthListener({ timeoutSecs: 1 });
      const callbackPromise = handle!.awaitCallback();
      // Let listen() register.
      await Promise.resolve();
      vi.advanceTimersByTime(1000);

      await expect(callbackPromise).rejects.toThrow('Loopback OAuth listener timed out');
      expect(unlisten).toHaveBeenCalledTimes(1);
      // Drain the queued microtask that calls stop().
      await Promise.resolve();
      expect(mockInvoke).toHaveBeenNthCalledWith(2, 'stop_loopback_oauth_listener');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('E2E build hook', () => {
  // Top-level side effect in loopbackOauthListener.ts: when the
  // VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD flag is set to 'true' at
  // build time, the module exposes startLoopbackOauthListener on
  // window.__startLoopbackOauthListener so E2E spec helpers can drive
  // the real loopback flow. Exercise both branches so the conditional
  // assignment is covered.

  type WithE2eHook = Window & { __startLoopbackOauthListener?: typeof startLoopbackOauthListener };

  test('exposes __startLoopbackOauthListener on window when the E2E build flag is set', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD', 'true');
    delete (window as WithE2eHook).__startLoopbackOauthListener;
    try {
      const mod = await import('../loopbackOauthListener');
      expect((window as WithE2eHook).__startLoopbackOauthListener).toBe(
        mod.startLoopbackOauthListener
      );
    } finally {
      vi.unstubAllEnvs();
      delete (window as WithE2eHook).__startLoopbackOauthListener;
    }
  });

  test('does NOT expose the hook when the E2E build flag is absent', async () => {
    vi.resetModules();
    vi.stubEnv('VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD', '');
    delete (window as WithE2eHook).__startLoopbackOauthListener;
    try {
      await import('../loopbackOauthListener');
      expect((window as WithE2eHook).__startLoopbackOauthListener).toBeUndefined();
    } finally {
      vi.unstubAllEnvs();
      delete (window as WithE2eHook).__startLoopbackOauthListener;
    }
  });
});
