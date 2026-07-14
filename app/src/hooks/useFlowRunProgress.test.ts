/**
 * useFlowRunProgress (Phase 3e) — unit tests.
 *
 * Verifies the hook builds a `node_id -> status` map from the socket
 * `flow:run_progress` feed, filters to the watched run, resets on a run change,
 * and unsubscribes on unmount. The socket is a tiny in-memory emitter so a
 * `flow:run_progress` event can be simulated deterministically.
 */
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useFlowRunProgress } from './useFlowRunProgress';

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
    for (const event of ['flow:run_progress', 'flow_run_progress']) {
      for (const cb of handlers.get(event) ?? []) cb(payload);
    }
  });
}

describe('useFlowRunProgress', () => {
  beforeEach(() => {
    handlers.clear();
    on.mockClear();
    off.mockClear();
  });

  it('returns an empty map and subscribes to nothing when runId is null', () => {
    const { result } = renderHook(() => useFlowRunProgress(null));
    expect(result.current).toEqual({});
    expect(on).not.toHaveBeenCalled();
  });

  it('accumulates node statuses for the watched run', () => {
    const { result } = renderHook(() => useFlowRunProgress('run_1'));
    expect(on).toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    expect(on).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));

    emit({ run_id: 'run_1', node_id: 'a', status: 'running' });
    expect(result.current).toEqual({ a: 'running' });

    emit({ run_id: 'run_1', node_id: 'a', status: 'success' });
    emit({ run_id: 'run_1', node_id: 'b', status: 'error' });
    expect(result.current).toEqual({ a: 'success', b: 'error' });
  });

  it('ignores events for other runs and malformed payloads', () => {
    const { result } = renderHook(() => useFlowRunProgress('run_1'));
    emit({ run_id: 'other', node_id: 'a', status: 'running' });
    emit({ node_id: 'a', status: 'running' }); // missing run_id
    emit(null);
    emit({ run_id: 'run_1', node_id: 'a' }); // missing status
    expect(result.current).toEqual({});
  });

  it('resets the map when the runId changes', () => {
    const { result, rerender } = renderHook(({ id }) => useFlowRunProgress(id), {
      initialProps: { id: 'run_1' as string | null },
    });
    emit({ run_id: 'run_1', node_id: 'a', status: 'success' });
    expect(result.current).toEqual({ a: 'success' });

    rerender({ id: 'run_2' });
    expect(result.current).toEqual({});
    emit({ run_id: 'run_1', node_id: 'a', status: 'success' }); // stale run ignored
    expect(result.current).toEqual({});
  });

  it('unsubscribes both event aliases on unmount', () => {
    const { unmount } = renderHook(() => useFlowRunProgress('run_1'));
    unmount();
    expect(off).toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    expect(off).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));
  });
});
