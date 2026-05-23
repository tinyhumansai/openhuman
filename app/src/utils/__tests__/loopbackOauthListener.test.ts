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
