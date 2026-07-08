/**
 * useFlowApprovalRequests (flow-approval surface — chat) — unit tests.
 *
 * Verifies the hook builds a de-duplicated list from the socket
 * `flow_approval_request` feed (both colon and underscore aliases), drops
 * malformed payloads, `dismiss()` removes a request by id, and it
 * unsubscribes on unmount. The socket is a tiny in-memory emitter so events
 * can be simulated deterministically — same harness as
 * `useFlowRunProgress.test.ts`.
 */
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useFlowApprovalRequests } from './useFlowApprovalRequests';

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

function emit(payload: unknown) {
  act(() => {
    for (const event of ['flow:approval_request', 'flow_approval_request']) {
      for (const cb of handlers.get(event) ?? []) cb(payload);
    }
  });
}

const REQUEST = {
  request_id: 'req-1',
  flow_id: 'flow-1',
  run_id: 'run-1',
  tool_name: 'shell',
  summary: 'Run `shell` — rm -rf /tmp/scratch',
};

describe('useFlowApprovalRequests', () => {
  beforeEach(() => {
    handlers.clear();
    on.mockClear();
    off.mockClear();
  });

  it('subscribes to both event aliases on mount', () => {
    renderHook(() => useFlowApprovalRequests());
    expect(on).toHaveBeenCalledWith('flow:approval_request', expect.any(Function));
    expect(on).toHaveBeenCalledWith('flow_approval_request', expect.any(Function));
  });

  it('adds a request from a valid payload', () => {
    const { result } = renderHook(() => useFlowApprovalRequests());
    emit(REQUEST);
    expect(result.current.requests).toEqual([REQUEST]);
  });

  it('de-duplicates by request_id', () => {
    const { result } = renderHook(() => useFlowApprovalRequests());
    emit(REQUEST);
    emit(REQUEST);
    expect(result.current.requests).toHaveLength(1);
  });

  it('ignores malformed payloads', () => {
    const { result } = renderHook(() => useFlowApprovalRequests());
    emit(null);
    emit({ ...REQUEST, request_id: undefined });
    emit({ ...REQUEST, flow_id: 42 });
    expect(result.current.requests).toEqual([]);
  });

  it('accumulates distinct requests', () => {
    const { result } = renderHook(() => useFlowApprovalRequests());
    emit(REQUEST);
    emit({ ...REQUEST, request_id: 'req-2' });
    expect(result.current.requests.map(r => r.request_id)).toEqual(['req-1', 'req-2']);
  });

  it('dismiss() removes a request by id', () => {
    const { result } = renderHook(() => useFlowApprovalRequests());
    emit(REQUEST);
    emit({ ...REQUEST, request_id: 'req-2' });
    expect(result.current.requests).toHaveLength(2);

    act(() => {
      result.current.dismiss('req-1');
    });
    expect(result.current.requests.map(r => r.request_id)).toEqual(['req-2']);
  });

  it('unsubscribes both event aliases on unmount', () => {
    const { unmount } = renderHook(() => useFlowApprovalRequests());
    unmount();
    expect(off).toHaveBeenCalledWith('flow:approval_request', expect.any(Function));
    expect(off).toHaveBeenCalledWith('flow_approval_request', expect.any(Function));
  });
});
