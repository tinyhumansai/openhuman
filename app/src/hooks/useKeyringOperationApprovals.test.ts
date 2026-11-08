import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useKeyringOperationApprovals } from './useKeyringOperationApprovals';

const handlers = vi.hoisted(() => new Map<string, Set<(data: unknown) => void>>());
const on = vi.hoisted(() =>
  vi.fn((event: string, cb: (data: unknown) => void) => {
    const set = handlers.get(event) ?? new Set();
    set.add(cb);
    handlers.set(event, set);
  })
);
const off = vi.hoisted(() =>
  vi.fn((event: string, cb: (data: unknown) => void) => {
    handlers.get(event)?.delete(cb);
  })
);
const isConnected = vi.hoisted(() => vi.fn(() => true));
vi.mock('../services/socketService', () => ({ socketService: { on, off, isConnected } }));
const listPending = vi.hoisted(() => vi.fn(async (): Promise<unknown[]> => []));
vi.mock('../services/keyringApi', () => ({ listPendingKeyringOperations: listPending }));

const EVENTS = ['keyring:access_approval_requested', 'keyring_access_approval_requested'];
const REQUEST = {
  request_id: 'request-1',
  caller_agent: 'OpenHuman',
  origin: 'local_operator',
  purpose: 'Read credential profile',
  access: 'read',
  name: 'auth:openai:default',
  expires_at_ms: Date.now() + 120_000,
};

function emit(payload: unknown) {
  act(() => {
    for (const event of EVENTS) {
      for (const callback of handlers.get(event) ?? []) callback(payload);
    }
  });
}

describe('useKeyringOperationApprovals', () => {
  beforeEach(() => {
    handlers.clear();
    on.mockClear();
    off.mockClear();
    isConnected.mockReset().mockReturnValue(true);
    listPending.mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('subscribes to both Socket.IO aliases', () => {
    renderHook(() => useKeyringOperationApprovals());
    for (const event of EVENTS) {
      expect(on).toHaveBeenCalledWith(event, expect.any(Function));
    }
  });

  it('adds a validated operation request and de-duplicates aliases', () => {
    const { result } = renderHook(() => useKeyringOperationApprovals());
    emit(REQUEST);

    expect(result.current.requests).toEqual([REQUEST]);
  });

  it('ignores malformed, unknown-access, and expired requests', () => {
    const { result } = renderHook(() => useKeyringOperationApprovals());
    emit(null);
    emit({ ...REQUEST, access: 'admin' });
    emit({ ...REQUEST, name: 42 });
    emit({ ...REQUEST, expires_at_ms: Date.now() - 1 });

    expect(result.current.requests).toEqual([]);
  });

  it('keeps distinct pending requests and dismisses by request id', () => {
    const { result } = renderHook(() => useKeyringOperationApprovals());
    emit(REQUEST);
    emit({ ...REQUEST, request_id: 'request-2', access: 'write' });
    expect(result.current.requests.map(request => request.request_id)).toEqual([
      'request-1',
      'request-2',
    ]);

    act(() => result.current.dismiss('request-1'));
    expect(result.current.requests.map(request => request.request_id)).toEqual(['request-2']);
  });

  it('unsubscribes both aliases on unmount', () => {
    const { unmount } = renderHook(() => useKeyringOperationApprovals());
    unmount();
    for (const event of EVENTS) {
      expect(off).toHaveBeenCalledWith(event, expect.any(Function));
    }
  });

  it('asks the core for pending requests while no socket is connected', async () => {
    vi.useFakeTimers();
    isConnected.mockReturnValue(false);
    listPending.mockResolvedValue([REQUEST]);
    const { result } = renderHook(() => useKeyringOperationApprovals());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });

    expect(listPending).toHaveBeenCalled();
    expect(result.current.requests).toEqual([REQUEST]);

    // The same request arriving over the socket later is not shown twice.
    emit(REQUEST);
    expect(result.current.requests).toEqual([REQUEST]);
  });

  it('does not poll once the socket is connected', async () => {
    vi.useFakeTimers();
    renderHook(() => useKeyringOperationApprovals());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_000);
    });

    expect(listPending).not.toHaveBeenCalled();
  });

  it('does not show a dismissed request again when a stale poll lists it', () => {
    const { result } = renderHook(() => useKeyringOperationApprovals());
    emit(REQUEST);
    act(() => result.current.dismiss(REQUEST.request_id));
    emit(REQUEST);

    expect(result.current.requests).toEqual([]);
  });
});
