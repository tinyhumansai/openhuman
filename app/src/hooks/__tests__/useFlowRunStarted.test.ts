/**
 * useFlowRunStarted — unit tests (issue B35).
 *
 * Verifies: subscribes to both socket event aliases unconditionally on mount,
 * invokes `onStart` for a matching payload, filters by `flowId` when
 * provided, passes every event through when `flowId` is omitted, drops
 * invalid payloads, and tears down both subscriptions on unmount.
 */
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useFlowRunStarted } from '../useFlowRunStarted';

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

function emit(event: 'flow:run_started' | 'flow_run_started', payload: unknown) {
  act(() => {
    for (const cb of handlers.get(event) ?? []) cb(payload);
  });
}

describe('useFlowRunStarted', () => {
  beforeEach(() => {
    handlers.clear();
    on.mockClear();
    off.mockClear();
  });

  it('subscribes to both event aliases unconditionally on mount', () => {
    renderHook(() => useFlowRunStarted(vi.fn()));

    expect(on).toHaveBeenCalledWith('flow:run_started', expect.any(Function));
    expect(on).toHaveBeenCalledWith('flow_run_started', expect.any(Function));
  });

  it('invokes onStart for a matching payload on either alias', () => {
    const onStart = vi.fn();
    renderHook(() => useFlowRunStarted(onStart));

    emit('flow:run_started', { flow_id: 'flow-1', run_id: 'run-1' });
    expect(onStart).toHaveBeenCalledWith({ flow_id: 'flow-1', run_id: 'run-1' });

    emit('flow_run_started', { flow_id: 'flow-2', run_id: 'run-2' });
    expect(onStart).toHaveBeenCalledWith({ flow_id: 'flow-2', run_id: 'run-2' });
    expect(onStart).toHaveBeenCalledTimes(2);
  });

  it('dedupes the colon and underscore aliases of the same run so onStart fires once', () => {
    // The core bridge re-emits one `FlowRunStarted` event under both socket
    // aliases with identical payloads — assert the hook collapses them.
    const onStart = vi.fn();
    renderHook(() => useFlowRunStarted(onStart));

    const payload = { flow_id: 'flow-1', run_id: 'run-1' };
    emit('flow:run_started', payload);
    emit('flow_run_started', payload);

    expect(onStart).toHaveBeenCalledTimes(1);
    expect(onStart).toHaveBeenCalledWith({ flow_id: 'flow-1', run_id: 'run-1' });

    // A genuinely different run still gets through.
    emit('flow:run_started', { flow_id: 'flow-1', run_id: 'run-2' });
    expect(onStart).toHaveBeenCalledTimes(2);
  });

  it('filters to the given flowId when provided', () => {
    const onStart = vi.fn();
    renderHook(() => useFlowRunStarted(onStart, 'flow-1'));

    emit('flow:run_started', { flow_id: 'flow-2', run_id: 'run-1' });
    expect(onStart).not.toHaveBeenCalled();

    emit('flow:run_started', { flow_id: 'flow-1', run_id: 'run-2' });
    expect(onStart).toHaveBeenCalledWith({ flow_id: 'flow-1', run_id: 'run-2' });
    expect(onStart).toHaveBeenCalledTimes(1);
  });

  it('passes every event through when flowId is omitted', () => {
    const onStart = vi.fn();
    renderHook(() => useFlowRunStarted(onStart));

    emit('flow:run_started', { flow_id: 'flow-1', run_id: 'run-1' });
    emit('flow:run_started', { flow_id: 'flow-2', run_id: 'run-2' });

    expect(onStart).toHaveBeenCalledTimes(2);
  });

  it('drops invalid payloads without invoking onStart', () => {
    const onStart = vi.fn();
    renderHook(() => useFlowRunStarted(onStart));

    emit('flow:run_started', null);
    emit('flow:run_started', {});
    emit('flow:run_started', { flow_id: 123, run_id: 'run-1' });
    emit('flow:run_started', { flow_id: 'flow-1', run_id: 456 });

    expect(onStart).not.toHaveBeenCalled();
  });

  it('cleans up both subscriptions on unmount', () => {
    const { unmount } = renderHook(() => useFlowRunStarted(vi.fn()));

    unmount();

    expect(off).toHaveBeenCalledWith('flow:run_started', expect.any(Function));
    expect(off).toHaveBeenCalledWith('flow_run_started', expect.any(Function));
  });
});
