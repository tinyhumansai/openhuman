/**
 * useFlowRunPoller (issue B3b) — poll-until-terminal contract.
 *
 * Asserts: initial loading→resolved, 3s poll cadence while `running` /
 * `pending_approval`, stop on `completed`/`failed`, stop when `runId` goes
 * `null`, error surfaced (and no further poll) on rejection, effect cleanup
 * on unmount, and (issue B35 follow-up) an immediate out-of-cadence tick
 * forced by a matching `FlowRunFinished` socket event.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import { useFlowRunPoller } from '../useFlowRunPoller';

const getFlowRun = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ getFlowRun }));

const handlers = vi.hoisted(() => new Map<string, Set<(data: unknown) => void>>());
const socketOn = vi.hoisted(() =>
  vi.fn((event: string, cb: (data: unknown) => void) => {
    const set = handlers.get(event) ?? new Set();
    set.add(cb);
    handlers.set(event, set);
  })
);
const socketOff = vi.hoisted(() =>
  vi.fn((event: string, cb: (data: unknown) => void) => {
    handlers.get(event)?.delete(cb);
  })
);
vi.mock('../../services/socketService', () => ({
  socketService: { on: socketOn, off: socketOff },
}));

function emitFinished(
  event: 'flow:run_finished' | 'flow_run_finished',
  payload: { flow_id: string; run_id: string; status: string }
) {
  for (const cb of handlers.get(event) ?? []) cb(payload);
}

function makeRun(overrides: Partial<FlowRun> = {}): FlowRun {
  return {
    id: 'thread-1',
    flow_id: 'flow-1',
    thread_id: 'thread-1',
    status: 'running',
    started_at: '2026-01-01T00:00:00Z',
    steps: [],
    pending_approvals: [],
    ...overrides,
  };
}

describe('useFlowRunPoller', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    handlers.clear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('starts in loading and resolves with the first fetched run', async () => {
    getFlowRun.mockResolvedValue(makeRun());
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    expect(result.current.loading).toBe(true);
    expect(result.current.run).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.run?.status).toBe('running');
    expect(result.current.error).toBeNull();
  });

  it('polls every 3s while the run is running', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'running' }));
    renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(3);
  });

  it('keeps polling while pending_approval (not terminal)', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'pending_approval' }));
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run?.status).toBe('pending_approval');
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(2);
  });

  it('stops polling once the run completes', async () => {
    getFlowRun.mockResolvedValue(
      makeRun({ status: 'completed', finished_at: '2026-01-01T00:01:00Z' })
    );
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run?.status).toBe('completed');
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('stops polling once the run completes with warnings', async () => {
    getFlowRun.mockResolvedValue(
      makeRun({ status: 'completed_with_warnings', finished_at: '2026-01-01T00:01:00Z' })
    );
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run?.status).toBe('completed_with_warnings');
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('stops polling once the run is cancelled', async () => {
    getFlowRun.mockResolvedValue(
      makeRun({ status: 'cancelled', finished_at: '2026-01-01T00:01:00Z' })
    );
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run?.status).toBe('cancelled');
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('stops polling once the run is interrupted', async () => {
    // Bug B42: an `interrupted` run (reconciled after being dropped mid-flight)
    // is terminal — the poller must not loop forever on it.
    getFlowRun.mockResolvedValue(
      makeRun({
        status: 'interrupted',
        error: 'Run interrupted before completion',
        finished_at: '2026-01-01T00:01:00Z',
      })
    );
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run?.status).toBe('interrupted');
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('stops polling once the run fails', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'failed', error: 'boom' }));
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run?.status).toBe('failed');

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('stops and clears state when runId becomes null', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'running' }));
    const { result, rerender } = renderHook(({ runId }) => useFlowRunPoller(runId), {
      initialProps: { runId: 'thread-1' as string | null },
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.run).not.toBeNull();

    rerender({ runId: null });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.run).toBeNull();
    expect(result.current.loading).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('sets error on rejection and does not schedule another poll', async () => {
    getFlowRun.mockRejectedValue(new Error('network down'));
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.error).toBe('network down');
    expect(result.current.loading).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('cleans up pending timers on unmount', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'running' }));
    const { unmount } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    unmount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    // No further calls after unmount.
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });

  it('does nothing when runId starts null', async () => {
    const { result } = renderHook(() => useFlowRunPoller(null));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.run).toBeNull();
    expect(getFlowRun).not.toHaveBeenCalled();
  });

  // ── FlowRunFinished-forced immediate tick (issue B35 follow-up) ─────────

  it('forces an immediate out-of-cadence tick and stops polling when a matching FlowRunFinished event arrives', async () => {
    getFlowRun.mockResolvedValueOnce(makeRun({ status: 'running' }));
    const { result } = renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);
    expect(result.current.run?.status).toBe('running');

    getFlowRun.mockResolvedValueOnce(
      makeRun({ status: 'completed', finished_at: '2026-01-01T00:01:00Z' })
    );

    // Fires well before the next scheduled (3s) poll tick would.
    await act(async () => {
      emitFinished('flow:run_finished', {
        flow_id: 'flow-1',
        run_id: 'thread-1',
        status: 'completed',
      });
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(getFlowRun).toHaveBeenCalledTimes(2);
    expect(result.current.run?.status).toBe('completed');

    // The scheduled poll from the first tick must have been cancelled by the
    // forced tick — no further calls once terminal, even well past 3s.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(2);
  });

  it('ignores a FlowRunFinished event for a different run', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'running' }));
    renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      emitFinished('flow_run_finished', {
        flow_id: 'flow-2',
        run_id: 'thread-2',
        status: 'completed',
      });
      await vi.advanceTimersByTimeAsync(0);
    });

    // No forced tick for an unrelated run — the regular 3s cadence still
    // governs.
    expect(getFlowRun).toHaveBeenCalledTimes(1);
  });
});
