/**
 * useFlowPendingApprovals (flow-approval surface — run details) — poll +
 * decide contract.
 *
 * Asserts: no-op while `flowId`/`runId` is null; polls
 * `approval_list_pending` every 2s while both are set; filters to gates
 * matching `source_context.{kind:"flow",flow_id,run_id}`; stops polling on a
 * fetch error (surfacing it) without hammering the endpoint; resets state
 * when `flowId`/`runId` change; `decide()` calls `decideApproval` and
 * optimistically drops the request on success, surfaces `error` and keeps it
 * in the list on failure; cleans up timers on unmount.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PendingApproval } from '../../services/api/approvalApi';
import { useFlowPendingApprovals } from '../useFlowPendingApprovals';

const fetchPendingApprovals = vi.hoisted(() => vi.fn());
const decideApproval = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/approvalApi', () => ({ fetchPendingApprovals, decideApproval }));

function makeApproval(overrides: Partial<PendingApproval> = {}): PendingApproval {
  return {
    request_id: 'req-1',
    tool_name: 'shell',
    action_summary: 'Run `shell`',
    args_redacted: {},
    session_id: 'session-1',
    created_at: '2026-01-01T00:00:00Z',
    expires_at: null,
    source_context: { kind: 'flow', flow_id: 'flow-1', run_id: 'run-1' },
    ...overrides,
  };
}

describe('useFlowPendingApprovals', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('does nothing when flowId or runId is null', async () => {
    const { result, rerender } = renderHook(
      ({ flowId, runId }: { flowId: string | null; runId: string | null }) =>
        useFlowPendingApprovals(flowId, runId),
      { initialProps: { flowId: null as string | null, runId: null as string | null } }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.approvals).toEqual([]);
    expect(fetchPendingApprovals).not.toHaveBeenCalled();

    rerender({ flowId: 'flow-1', runId: null });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchPendingApprovals).not.toHaveBeenCalled();
  });

  it('polls every 2s and filters to this flow/run via source_context', async () => {
    fetchPendingApprovals.mockResolvedValue([
      makeApproval({ request_id: 'req-a' }),
      makeApproval({
        request_id: 'req-b',
        source_context: { kind: 'flow', flow_id: 'flow-2', run_id: 'run-1' },
      }),
      makeApproval({ request_id: 'req-c', source_context: undefined }),
    ]);
    const { result } = renderHook(() => useFlowPendingApprovals('flow-1', 'run-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.approvals.map(a => a.request_id)).toEqual(['req-a']);
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(2);
  });

  it('surfaces an error and stops polling on a failed fetch', async () => {
    fetchPendingApprovals.mockRejectedValue(new Error('network down'));
    const { result } = renderHook(() => useFlowPendingApprovals('flow-1', 'run-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.error).toBe('network down');

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);
  });

  it('resets approvals when flowId/runId change', async () => {
    fetchPendingApprovals.mockResolvedValue([makeApproval({ request_id: 'req-a' })]);
    const { result, rerender } = renderHook(
      ({ flowId, runId }: { flowId: string | null; runId: string | null }) =>
        useFlowPendingApprovals(flowId, runId),
      { initialProps: { flowId: 'flow-1' as string | null, runId: 'run-1' as string | null } }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.approvals).toHaveLength(1);

    rerender({ flowId: null, runId: null });
    expect(result.current.approvals).toEqual([]);
  });

  it('decide() calls decideApproval and optimistically drops the request on success', async () => {
    fetchPendingApprovals.mockResolvedValue([makeApproval({ request_id: 'req-a' })]);
    decideApproval.mockResolvedValue(undefined);
    const { result } = renderHook(() => useFlowPendingApprovals('flow-1', 'run-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.approvals).toHaveLength(1);

    await act(async () => {
      await result.current.decide('req-a', 'approve_once');
    });

    expect(decideApproval).toHaveBeenCalledWith('req-a', 'approve_once');
    expect(result.current.approvals).toEqual([]);
    expect(result.current.decidingId).toBeNull();
  });

  it('decide() surfaces an error and keeps the request when the RPC fails', async () => {
    fetchPendingApprovals.mockResolvedValue([makeApproval({ request_id: 'req-a' })]);
    decideApproval.mockRejectedValue(new Error('gate not installed'));
    const { result } = renderHook(() => useFlowPendingApprovals('flow-1', 'run-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    await act(async () => {
      await expect(result.current.decide('req-a', 'deny')).rejects.toThrow('gate not installed');
    });

    expect(result.current.error).toBe('gate not installed');
    expect(result.current.approvals).toHaveLength(1);
    expect(result.current.decidingId).toBeNull();
  });

  it('cleans up pending timers on unmount', async () => {
    fetchPendingApprovals.mockResolvedValue([]);
    const { unmount } = renderHook(() => useFlowPendingApprovals('flow-1', 'run-1'));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    unmount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);
  });
});
