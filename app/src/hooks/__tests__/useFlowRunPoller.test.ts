/**
 * useFlowRunPoller (issue B3b) — poll-until-terminal contract.
 *
 * Asserts: initial loading→resolved, 2s poll cadence while `running` /
 * `pending_approval`, stop on `completed`/`failed`, stop when `runId` goes
 * `null`, error surfaced (and no further poll) on rejection, and effect
 * cleanup on unmount.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import { useFlowRunPoller } from '../useFlowRunPoller';

const getFlowRun = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ getFlowRun }));

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

  it('polls every 2s while the run is running', async () => {
    getFlowRun.mockResolvedValue(makeRun({ status: 'running' }));
    renderHook(() => useFlowRunPoller('thread-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(getFlowRun).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
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
      await vi.advanceTimersByTimeAsync(2000);
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
});
