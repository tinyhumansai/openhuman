/**
 * useFlowRunsLiveRefresh — unit tests.
 *
 * Verifies: no subscription/poll when every run is terminal, subscribe +
 * poll when a run is active, a trailing-debounced refetch on a matching
 * `flow:run_progress`/`flow_run_progress` event, teardown once the runs
 * settle to all-terminal, and cleanup on unmount.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import { useFlowRunsLiveRefresh } from '../useFlowRunsLiveRefresh';

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
vi.mock('../../services/socketService', () => ({ socketService: { on, off } }));

function emit(event: 'flow:run_progress' | 'flow_run_progress', payload: unknown) {
  act(() => {
    for (const cb of handlers.get(event) ?? []) cb(payload);
  });
}

function makeRun(overrides: Partial<FlowRun> = {}): FlowRun {
  return {
    id: 'run-1',
    flow_id: 'flow-1',
    thread_id: 'run-1',
    status: 'running',
    started_at: '2026-01-01T00:00:00Z',
    steps: [],
    pending_approvals: [],
    ...overrides,
  };
}

describe('useFlowRunsLiveRefresh', () => {
  beforeEach(() => {
    handlers.clear();
    on.mockClear();
    off.mockClear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('does not subscribe or poll when every run is terminal', () => {
    const refetch = vi.fn();
    renderHook(() =>
      useFlowRunsLiveRefresh(
        [makeRun({ status: 'completed' }), makeRun({ id: 'run-2', status: 'failed' })],
        refetch
      )
    );

    expect(on).not.toHaveBeenCalled();

    vi.advanceTimersByTime(20_000);
    expect(refetch).not.toHaveBeenCalled();
  });

  it('subscribes to both event aliases and polls on a fallback interval when a run is active', () => {
    const refetch = vi.fn();
    renderHook(() => useFlowRunsLiveRefresh([makeRun({ status: 'running' })], refetch));

    expect(on).toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    expect(on).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));

    vi.advanceTimersByTime(30_000);
    expect(refetch).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(30_000);
    expect(refetch).toHaveBeenCalledTimes(2);
  });

  it('debounces a burst of flow:run_progress events into a single trailing refetch', () => {
    const refetch = vi.fn();
    renderHook(() => useFlowRunsLiveRefresh([makeRun({ status: 'running' })], refetch));

    // Keep every emit + the final debounce settle comfortably inside the
    // first 30s poll tick so the poll fallback doesn't also fire here — that
    // interplay is covered separately by the "poll fallback" test above.
    emit('flow:run_progress', { run_id: 'run-1', node_id: 'a', status: 'success' });
    vi.advanceTimersByTime(500);
    emit('flow:run_progress', { run_id: 'run-1', node_id: 'b', status: 'success' });
    vi.advanceTimersByTime(500);
    emit('flow_run_progress', { run_id: 'run-1', node_id: 'c', status: 'success' });

    // Still within the 3s trailing window from the last event — no refetch yet.
    expect(refetch).not.toHaveBeenCalled();

    vi.advanceTimersByTime(3_000);
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it('tears down the subscription and poll once the runs settle to all-terminal', () => {
    const refetch = vi.fn();
    const { rerender } = renderHook(({ runs }) => useFlowRunsLiveRefresh(runs, refetch), {
      initialProps: { runs: [makeRun({ status: 'running' })] },
    });

    expect(on).toHaveBeenCalledTimes(2);

    rerender({ runs: [makeRun({ status: 'completed' })] });

    expect(off).toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    expect(off).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));

    refetch.mockClear();
    vi.advanceTimersByTime(20_000);
    expect(refetch).not.toHaveBeenCalled();
  });

  it('cleans up the subscription, poll, and any pending debounce on unmount', () => {
    const refetch = vi.fn();
    const { unmount } = renderHook(() =>
      useFlowRunsLiveRefresh([makeRun({ status: 'running' })], refetch)
    );

    emit('flow:run_progress', { run_id: 'run-1', node_id: 'a', status: 'success' });

    unmount();

    expect(off).toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    expect(off).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));

    refetch.mockClear();
    vi.advanceTimersByTime(20_000);
    expect(refetch).not.toHaveBeenCalled();
  });
});
