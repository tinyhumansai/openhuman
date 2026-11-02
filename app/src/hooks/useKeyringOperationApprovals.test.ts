import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

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
vi.mock('../services/socketService', () => ({ socketService: { on, off } }));

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
});
