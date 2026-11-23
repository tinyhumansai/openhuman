import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { CoreRpcError } from '../services/coreRpcClient';
import { CoreStateContext, type CoreStateContextValue } from './coreStateContext';
import type { DictationErrorCode, DictationSnapshot } from './dictationAdapter';
import { useComposerDictation } from './useComposerDictation';

const mocks = vi.hoisted(() => ({
  callCoreRpc: vi.fn(),
  captureSupported: vi.fn(),
  createAdapter: vi.fn(),
  invalidationListeners: new Set<() => void>(),
}));

vi.mock('../services/coreRpcClient', () => ({
  callCoreRpc: mocks.callCoreRpc,
  CoreRpcError: class extends Error {
    constructor(
      message: string,
      readonly kind: string
    ) {
      super(message);
    }
  },
  subscribeCoreRpcTokenInvalidated: (listener: () => void) => {
    mocks.invalidationListeners.add(listener);
    return () => mocks.invalidationListeners.delete(listener);
  },
}));
// Capability/error projection is isolated here. dictationAdapter.fallback.test.ts
// drives the real adapter's retry, cancellation, and conversion deadlines.
vi.mock('./dictationAdapter', () => ({
  createOpenHumanDictationAdapter: mocks.createAdapter,
  isDictationCaptureSupported: mocks.captureSupported,
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeAdapter() {
  let snapshot: DictationSnapshot = { phase: 'idle', error: null };
  const listeners = new Set<() => void>();
  return {
    getSnapshot: () => snapshot,
    subscribe: vi.fn((listener: () => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    }),
    cancel: vi.fn(),
    dispose: vi.fn(),
    publish(next: DictationSnapshot) {
      snapshot = next;
      for (const listener of listeners) listener();
    },
    get subscriberCount() {
      return listeners.size;
    },
  };
}

type FakeAdapter = ReturnType<typeof makeAdapter>;
let adapters: FakeAdapter[];

beforeEach(() => {
  mocks.callCoreRpc.mockReset().mockResolvedValue({ stt_available: true });
  mocks.captureSupported.mockReset().mockReturnValue(true);
  mocks.createAdapter.mockReset().mockImplementation(() => {
    const adapter = makeAdapter();
    adapters.push(adapter);
    return adapter;
  });
  adapters = [];
});

afterEach(() => {
  cleanup();
  mocks.invalidationListeners.clear();
  vi.useRealTimers();
});

describe('useComposerDictation', () => {
  it('retries transient probe failures with bounded exponential backoff', async () => {
    vi.useFakeTimers();
    mocks.callCoreRpc.mockRejectedValue(new Error('temporary failure'));
    const { result } = renderHook(() => useComposerDictation('thread-a'));
    await act(async () => {});
    expect(result.current.error).toBe('voice-status-failed');
    for (const delay of [1_000, 2_000, 4_000]) {
      const calls = mocks.callCoreRpc.mock.calls.length;
      await act(async () => vi.advanceTimersByTimeAsync(delay - 1));
      expect(mocks.callCoreRpc).toHaveBeenCalledTimes(calls);
      await act(async () => vi.advanceTimersByTimeAsync(1));
      expect(mocks.callCoreRpc).toHaveBeenCalledTimes(calls + 1);
    }
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.callCoreRpc).toHaveBeenCalledTimes(4);
    expect(result.current.adapter).toBeUndefined();

    mocks.callCoreRpc.mockResolvedValue({ stt_available: true });
    await act(async () => window.dispatchEvent(new Event('focus')));
    expect(result.current.adapter).toBe(adapters[0]);
    expect(result.current.error).toBeNull();
  });

  it('recovers on a scheduled retry and stops retrying once available', async () => {
    vi.useFakeTimers();
    mocks.callCoreRpc.mockRejectedValueOnce(new Error('core starting'));
    const { result } = renderHook(() => useComposerDictation('thread-a'));
    await act(async () => {});
    await act(async () => vi.advanceTimersByTimeAsync(1_000));
    expect(result.current.adapter).toBe(adapters[0]);
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.callCoreRpc).toHaveBeenCalledTimes(2);
  });

  it.each(['unconfigured', 'missing-method'])(
    'does not automatically retry %s but rechecks on focus',
    async condition => {
      vi.useFakeTimers();
      if (condition === 'missing-method') {
        mocks.callCoreRpc.mockRejectedValue(new CoreRpcError('missing', 'method_not_found'));
      } else {
        mocks.callCoreRpc.mockResolvedValue({ stt_available: false });
      }
      const { result } = renderHook(() => useComposerDictation('thread-a'));
      await act(async () => {});
      await act(async () => vi.advanceTimersByTimeAsync(60_000));
      expect(result.current.error).toBe('stt-unavailable');
      expect(mocks.callCoreRpc).toHaveBeenCalledOnce();

      mocks.callCoreRpc.mockResolvedValue({ stt_available: true });
      await act(async () => window.dispatchEvent(new Event('focus')));
      expect(result.current.adapter).toBe(adapters[0]);
      expect(result.current.error).toBeNull();
    }
  );

  it('coalesces focus probes and preserves an existing capture session', async () => {
    const probe = deferred<{ stt_available: boolean }>();
    mocks.callCoreRpc.mockReturnValue(probe.promise);
    const { result, rerender } = renderHook(() => useComposerDictation('thread-a'));
    act(() => window.dispatchEvent(new Event('focus')));
    expect(mocks.callCoreRpc).toHaveBeenCalledOnce();
    await act(async () => probe.resolve({ stt_available: true }));
    act(() => adapters[0]!.publish({ phase: 'recording', error: null }));
    const state = result.current;
    rerender();
    expect(result.current).toBe(state);
    act(() => window.dispatchEvent(new Event('focus')));
    expect(mocks.callCoreRpc).toHaveBeenCalledOnce();
    expect(adapters[0]!.cancel).not.toHaveBeenCalled();
    expect(adapters[0]!.dispose).not.toHaveBeenCalled();
  });

  it('cancels retry timers on scope changes and all listeners on unmount', async () => {
    vi.useFakeTimers();
    mocks.callCoreRpc.mockRejectedValueOnce(new Error('temporary failure'));
    const { result, rerender, unmount } = renderHook(({ thread }) => useComposerDictation(thread), {
      initialProps: { thread: 'thread-a' },
    });
    await act(async () => {});
    rerender({ thread: 'thread-b' });
    await act(async () => {});
    expect(result.current.adapter).toBe(adapters[0]);
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.callCoreRpc).toHaveBeenCalledTimes(2);

    mocks.callCoreRpc.mockRejectedValue(new Error('temporary failure'));
    rerender({ thread: 'thread-c' });
    await act(async () => {});
    unmount();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(mocks.callCoreRpc).toHaveBeenCalledTimes(3);
  });

  it('fails closed until the active core confirms a usable STT provider', async () => {
    const probe = deferred<{ stt_available: boolean }>();
    mocks.callCoreRpc.mockReturnValue(probe.promise);
    const { result } = renderHook(() => useComposerDictation('thread-a'));

    expect(result.current.adapter).toBeUndefined();
    expect(result.current.status).toBe('checking');
    expect(mocks.createAdapter).not.toHaveBeenCalled();
    expect(mocks.callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.voice_status',
      params: {},
      suppressAuthExpiredEvent: true,
    });
    await act(async () => probe.resolve({ stt_available: true }));

    expect(result.current.adapter).toBe(adapters[0]);
    expect(result.current.status).toBe('idle');
    expect(result.current.error).toBeNull();
  });

  it('does not probe or offer dictation when browser capture is unsupported', () => {
    mocks.captureSupported.mockReturnValue(false);
    const { result } = renderHook(() => useComposerDictation('thread-a'));

    expect(result.current.adapter).toBeUndefined();
    expect(result.current.status).toBe('unavailable');
    expect(result.current.error).toBeNull();
    expect(mocks.callCoreRpc).not.toHaveBeenCalled();
  });

  it('does not offer capture before a thread exists', () => {
    const { result } = renderHook(() => useComposerDictation(null));

    expect(result.current.adapter).toBeUndefined();
    expect(result.current.status).toBe('unavailable');
    expect(result.current.error).toBeNull();
    expect(mocks.callCoreRpc).not.toHaveBeenCalled();
  });

  it('withholds dictation when the configured engine is unavailable', async () => {
    mocks.callCoreRpc.mockResolvedValue({ stt_available: false, stt_error: 'private detail' });
    const { result } = renderHook(() => useComposerDictation('thread-a'));

    await waitFor(() => expect(result.current.status).toBe('unavailable'));
    expect(result.current.adapter).toBeUndefined();
    expect(result.current.error).toBe('stt-unavailable');
    expect(mocks.createAdapter).not.toHaveBeenCalled();
  });

  it('fails closed on a status probe failure without exposing RPC message text', async () => {
    mocks.callCoreRpc.mockRejectedValue(new Error('private provider detail'));
    const { result } = renderHook(() => useComposerDictation('thread-a'));

    await waitFor(() => expect(result.current.status).toBe('unavailable'));
    expect(result.current.adapter).toBeUndefined();
    expect(result.current.error).toBe('voice-status-failed');
  });

  it('quietly withholds capability for a core built without the voice status method', async () => {
    mocks.callCoreRpc.mockRejectedValue(
      new CoreRpcError('unknown method: openhuman.voice_status', 'method_not_found')
    );
    const { result } = renderHook(() => useComposerDictation('thread-a'));

    await waitFor(() => expect(result.current.status).toBe('unavailable'));
    expect(result.current.adapter).toBeUndefined();
    expect(result.current.error).toBe('stt-unavailable');
  });

  it('publishes capture and transcription phases', async () => {
    const { result } = renderHook(() => useComposerDictation('thread-a'));
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    const adapter = adapters[0]!;

    for (const phase of ['starting', 'recording', 'transcribing'] as const) {
      act(() => adapter.publish({ phase, error: null }));
      expect(result.current.status).toBe(phase);
    }
  });

  // Keep this exhaustive as the adapter adds recoverable errors. A missing voice
  // domain withdraws the capability instead and is covered separately below.
  const recoverableErrors = {
    'microphone-unavailable': true,
    'permission-denied': true,
    'device-unavailable': true,
    'device-in-use': true,
    'recorder-failed': true,
    'no-audio': true,
    'no-speech': true,
    'transcription-failed': true,
    'timed-out': true,
  } satisfies Record<Exclude<DictationErrorCode, 'voice-unavailable'>, true>;

  it.each(Object.keys(recoverableErrors) as (keyof typeof recoverableErrors)[])(
    'publishes and clears %s while retaining the adapter for retry',
    async error => {
      const { result } = renderHook(() => useComposerDictation('thread-a'));
      await waitFor(() => expect(result.current.adapter).toBeDefined());
      const adapter = adapters[0]!;

      act(() => adapter.publish({ phase: 'idle', error }));
      expect(result.current.status).toBe('idle');
      expect(result.current.error).toBe(error);
      expect(result.current.adapter).toBe(adapter);
      expect(adapter.cancel).not.toHaveBeenCalled();
      expect(adapter.dispose).not.toHaveBeenCalled();

      act(() => adapter.publish({ phase: 'recording', error: null }));
      expect(result.current.status).toBe('recording');
      expect(result.current.error).toBeNull();
      expect(result.current.adapter).toBe(adapter);
    }
  );

  it('withdraws and cancels dictation when STT discovers the voice domain is absent', async () => {
    const { result } = renderHook(() => useComposerDictation('thread-a'));
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    const adapter = adapters[0]!;

    act(() => adapter.publish({ phase: 'idle', error: 'voice-unavailable' }));
    expect(result.current.adapter).toBeUndefined();
    expect(result.current.status).toBe('unavailable');
    expect(result.current.error).toBe('voice-unavailable');
    expect(adapter.cancel).toHaveBeenCalledOnce();
    expect(adapter.dispose).toHaveBeenCalledOnce();
    expect(adapter.subscriberCount).toBe(0);
    act(() => adapter.publish({ phase: 'idle', error: null }));
    expect(result.current.adapter).toBeUndefined();
  });

  it('cancels the active session on request without withdrawing the capability', async () => {
    const { result } = renderHook(() => useComposerDictation('thread-a'));
    await waitFor(() => expect(result.current.adapter).toBeDefined());

    act(() => result.current.cancel());
    expect(adapters[0]!.cancel).toHaveBeenCalledOnce();
    expect(result.current.adapter).toBe(adapters[0]);
  });

  it('disposes the previous thread before probing and exposing a new adapter', async () => {
    const { result, rerender } = renderHook(({ threadId }) => useComposerDictation(threadId), {
      initialProps: { threadId: 'thread-a' },
    });
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    const old = adapters[0]!;
    const nextProbe = deferred<{ stt_available: boolean }>();
    mocks.callCoreRpc.mockReturnValueOnce(nextProbe.promise);

    rerender({ threadId: 'thread-b' });
    expect(old.dispose).toHaveBeenCalledOnce();
    expect(old.subscriberCount).toBe(0);
    expect(result.current.adapter).toBeUndefined();
    expect(result.current.status).toBe('checking');
    act(() => old.publish({ phase: 'idle', error: 'voice-unavailable' }));
    expect(result.current.error).toBeNull();
    await act(async () => nextProbe.resolve({ stt_available: true }));
    expect(result.current.adapter).toBe(adapters[1]);
  });

  it('ignores a capability probe arriving after a thread switch', async () => {
    const oldProbe = deferred<{ stt_available: boolean }>();
    const nextProbe = deferred<{ stt_available: boolean }>();
    mocks.callCoreRpc.mockReturnValueOnce(oldProbe.promise).mockReturnValueOnce(nextProbe.promise);
    const { result, rerender } = renderHook(({ threadId }) => useComposerDictation(threadId), {
      initialProps: { threadId: 'thread-a' },
    });

    rerender({ threadId: 'thread-b' });
    await act(async () => oldProbe.resolve({ stt_available: true }));
    expect(mocks.createAdapter).not.toHaveBeenCalled();
    expect(result.current.adapter).toBeUndefined();
    await act(async () => nextProbe.resolve({ stt_available: false }));
    expect(result.current.error).toBe('stt-unavailable');
  });

  it('reprobes after bearer invalidation even if the thread and URL are unchanged', async () => {
    const { result } = renderHook(() => useComposerDictation('thread-a'));
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    const old = adapters[0]!;
    const nextProbe = deferred<{ stt_available: boolean }>();
    mocks.callCoreRpc.mockReturnValueOnce(nextProbe.promise);

    act(() => {
      for (const invalidate of mocks.invalidationListeners) invalidate();
    });
    expect(old.cancel).toHaveBeenCalledOnce();
    expect(old.dispose).toHaveBeenCalledOnce();
    expect(result.current.adapter).toBeUndefined();
    await act(async () => nextProbe.resolve({ stt_available: false }));
    expect(result.current.status).toBe('unavailable');
    expect(result.current.adapter).toBeUndefined();
    expect(mocks.callCoreRpc).toHaveBeenCalledTimes(2);
  });

  it('does not reuse capability or capture across credential and profile changes', async () => {
    let context = {
      snapshot: { auth: { userId: 'user-a', profileId: 'profile-a' }, sessionToken: 'session-a' },
    } as unknown as CoreStateContextValue;
    const wrapper = ({ children }: { children: ReactNode }) => (
      <CoreStateContext.Provider value={context}>{children}</CoreStateContext.Provider>
    );
    const { result, rerender } = renderHook(() => useComposerDictation('thread-a'), { wrapper });
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    const old = adapters[0]!;
    mocks.callCoreRpc.mockResolvedValueOnce({ stt_available: false });
    context = {
      snapshot: { auth: { userId: 'user-b', profileId: 'profile-b' }, sessionToken: 'session-b' },
    } as unknown as CoreStateContextValue;

    rerender();
    expect(old.dispose).toHaveBeenCalledOnce();
    expect(result.current.adapter).toBeUndefined();
    await waitFor(() => expect(result.current.error).toBe('stt-unavailable'));
  });

  it('disposes active capture if browser support becomes unavailable', async () => {
    const { result, rerender } = renderHook(() => useComposerDictation('thread-a'));
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    mocks.captureSupported.mockReturnValue(false);

    rerender();
    expect(adapters[0]!.dispose).toHaveBeenCalledOnce();
    expect(result.current.adapter).toBeUndefined();
    expect(result.current.error).toBeNull();
  });

  it('releases subscriptions and capture on unmount', async () => {
    const { result, unmount } = renderHook(() => useComposerDictation('thread-a'));
    await waitFor(() => expect(result.current.adapter).toBeDefined());
    unmount();

    expect(adapters[0]!.dispose).toHaveBeenCalledOnce();
    expect(adapters[0]!.subscriberCount).toBe(0);
    expect(mocks.invalidationListeners.size).toBe(0);
  });

  it('ignores a status probe that resolves after unmount', async () => {
    const probe = deferred<{ stt_available: boolean }>();
    mocks.callCoreRpc.mockReturnValueOnce(probe.promise);
    const { unmount } = renderHook(() => useComposerDictation('thread-a'));
    unmount();

    await act(async () => probe.resolve({ stt_available: true }));
    expect(mocks.createAdapter).not.toHaveBeenCalled();
  });
});
