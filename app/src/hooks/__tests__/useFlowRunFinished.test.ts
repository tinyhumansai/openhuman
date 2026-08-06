/**
 * useFlowRunFinished — unit tests (issue B35 follow-up).
 *
 * Verifies: subscribes to both socket event aliases unconditionally on mount,
 * invokes `onFinish` for a matching payload, filters by `flowId` when
 * provided, passes every event through when `flowId` is omitted, drops
 * invalid payloads, dedupes the colon/underscore alias replay, and tears down
 * both subscriptions on unmount.
 */
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useFlowRunFinished } from '../useFlowRunFinished';

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

function emit(event: 'flow:run_finished' | 'flow_run_finished', payload: unknown) {
  act(() => {
    for (const cb of handlers.get(event) ?? []) cb(payload);
  });
}

describe('useFlowRunFinished', () => {
  beforeEach(() => {
    handlers.clear();
    on.mockClear();
    off.mockClear();
  });

  it('subscribes to both event aliases unconditionally on mount', () => {
    renderHook(() => useFlowRunFinished(vi.fn()));

    expect(on).toHaveBeenCalledWith('flow:run_finished', expect.any(Function));
    expect(on).toHaveBeenCalledWith('flow_run_finished', expect.any(Function));
  });

  it('invokes onFinish for a matching payload on either alias', () => {
    const onFinish = vi.fn();
    renderHook(() => useFlowRunFinished(onFinish));

    emit('flow:run_finished', { flow_id: 'flow-1', run_id: 'run-1', status: 'completed' });
    expect(onFinish).toHaveBeenCalledWith({
      flow_id: 'flow-1',
      run_id: 'run-1',
      status: 'completed',
    });

    emit('flow_run_finished', { flow_id: 'flow-2', run_id: 'run-2', status: 'failed' });
    expect(onFinish).toHaveBeenCalledWith({ flow_id: 'flow-2', run_id: 'run-2', status: 'failed' });
    expect(onFinish).toHaveBeenCalledTimes(2);
  });

  it('dedupes the colon and underscore aliases of the same run so onFinish fires once', () => {
    // The core bridge re-emits one `FlowRunFinished` event under both socket
    // aliases with identical payloads — assert the hook collapses them.
    const onFinish = vi.fn();
    renderHook(() => useFlowRunFinished(onFinish));

    const payload = { flow_id: 'flow-1', run_id: 'run-1', status: 'completed' };
    emit('flow:run_finished', payload);
    emit('flow_run_finished', payload);

    expect(onFinish).toHaveBeenCalledTimes(1);
    expect(onFinish).toHaveBeenCalledWith({
      flow_id: 'flow-1',
      run_id: 'run-1',
      status: 'completed',
    });

    // A genuinely different run still gets through.
    emit('flow:run_finished', { flow_id: 'flow-1', run_id: 'run-2', status: 'failed' });
    expect(onFinish).toHaveBeenCalledTimes(2);
  });

  it('filters to the given flowId when provided', () => {
    const onFinish = vi.fn();
    renderHook(() => useFlowRunFinished(onFinish, 'flow-1'));

    emit('flow:run_finished', { flow_id: 'flow-2', run_id: 'run-1', status: 'completed' });
    expect(onFinish).not.toHaveBeenCalled();

    emit('flow:run_finished', { flow_id: 'flow-1', run_id: 'run-2', status: 'completed' });
    expect(onFinish).toHaveBeenCalledWith({
      flow_id: 'flow-1',
      run_id: 'run-2',
      status: 'completed',
    });
    expect(onFinish).toHaveBeenCalledTimes(1);
  });

  it('passes every event through when flowId is omitted', () => {
    const onFinish = vi.fn();
    renderHook(() => useFlowRunFinished(onFinish));

    emit('flow:run_finished', { flow_id: 'flow-1', run_id: 'run-1', status: 'completed' });
    emit('flow:run_finished', { flow_id: 'flow-2', run_id: 'run-2', status: 'failed' });

    expect(onFinish).toHaveBeenCalledTimes(2);
  });

  it('drops invalid payloads without invoking onFinish', () => {
    const onFinish = vi.fn();
    renderHook(() => useFlowRunFinished(onFinish));

    emit('flow:run_finished', null);
    emit('flow:run_finished', {});
    emit('flow:run_finished', { flow_id: 123, run_id: 'run-1', status: 'completed' });
    emit('flow:run_finished', { flow_id: 'flow-1', run_id: 456, status: 'completed' });
    emit('flow:run_finished', { flow_id: 'flow-1', run_id: 'run-1', status: 42 });

    expect(onFinish).not.toHaveBeenCalled();
  });

  it('cleans up both subscriptions on unmount', () => {
    const { unmount } = renderHook(() => useFlowRunFinished(vi.fn()));

    unmount();

    expect(off).toHaveBeenCalledWith('flow:run_finished', expect.any(Function));
    expect(off).toHaveBeenCalledWith('flow_run_finished', expect.any(Function));
  });
});
