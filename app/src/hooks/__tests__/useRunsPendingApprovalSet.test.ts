/**
 * useRunsPendingApprovalSet — unit tests.
 *
 * Verifies: no poll when every run is terminal (or the list is empty); polls
 * `approval_list_pending` every 3s while a run is `running`; collects only
 * flow-origin (`source_context.kind === 'flow'`) matches into the returned
 * Set; a chat-origin approval (no flow `source_context`) is excluded; a
 * failed poll keeps the last-known set instead of clearing it; teardown once
 * every run settles to terminal; cleanup on unmount.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PendingApproval } from '../../services/api/approvalApi';
import type { FlowRun } from '../../services/api/flowsApi';
import { resolveDisplayStatus, useRunsPendingApprovalSet } from '../useRunsPendingApprovalSet';

const fetchPendingApprovals = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/approvalApi', () => ({ fetchPendingApprovals }));

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

describe('useRunsPendingApprovalSet', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('does not poll when every run is terminal', async () => {
    fetchPendingApprovals.mockResolvedValue([]);
    renderHook(() =>
      useRunsPendingApprovalSet([
        makeRun({ status: 'completed' }),
        makeRun({ id: 'run-2', status: 'failed' }),
      ])
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(fetchPendingApprovals).not.toHaveBeenCalled();
  });

  it('does not poll for an empty runs list', async () => {
    fetchPendingApprovals.mockResolvedValue([]);
    renderHook(() => useRunsPendingApprovalSet([]));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(fetchPendingApprovals).not.toHaveBeenCalled();
  });

  it('polls every 3s while a run is running and includes it when a matching flow approval exists', async () => {
    fetchPendingApprovals.mockResolvedValue([makeApproval({ request_id: 'req-a' })]);
    const { result } = renderHook(() =>
      useRunsPendingApprovalSet([makeRun({ status: 'running' })])
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);
    expect(result.current.has('run-1')).toBe(true);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(2);
  });

  it('excludes a running run with no matching pending approval', async () => {
    fetchPendingApprovals.mockResolvedValue([]);
    const { result } = renderHook(() =>
      useRunsPendingApprovalSet([makeRun({ status: 'running' })])
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.size).toBe(0);
  });

  it('excludes a chat-origin approval (no flow source_context)', async () => {
    fetchPendingApprovals.mockResolvedValue([
      makeApproval({ request_id: 'req-chat', source_context: undefined }),
    ]);
    const { result } = renderHook(() =>
      useRunsPendingApprovalSet([makeRun({ status: 'running' })])
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.size).toBe(0);
  });

  it('keeps the last-known set when a poll fails', async () => {
    fetchPendingApprovals.mockResolvedValueOnce([makeApproval({ request_id: 'req-a' })]);
    const { result, rerender } = renderHook(
      ({ runs }: { runs: FlowRun[] }) => useRunsPendingApprovalSet(runs),
      { initialProps: { runs: [makeRun({ status: 'running' })] } }
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.has('run-1')).toBe(true);

    fetchPendingApprovals.mockRejectedValueOnce(new Error('network down'));
    rerender({ runs: [makeRun({ status: 'running' })] });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(2);
    expect(result.current.has('run-1')).toBe(true);

    // A subsequent successful tick keeps polling on the same cadence.
    fetchPendingApprovals.mockResolvedValueOnce([makeApproval({ request_id: 'req-a' })]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(3);
  });

  it('tears down polling once every run settles to terminal', async () => {
    fetchPendingApprovals.mockResolvedValue([makeApproval({ request_id: 'req-a' })]);
    const { rerender } = renderHook(
      ({ runs }: { runs: FlowRun[] }) => useRunsPendingApprovalSet(runs),
      { initialProps: { runs: [makeRun({ status: 'running' })] } }
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    rerender({ runs: [makeRun({ status: 'completed' })] });

    fetchPendingApprovals.mockClear();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(fetchPendingApprovals).not.toHaveBeenCalled();
  });

  it('cleans up pending timers on unmount', async () => {
    fetchPendingApprovals.mockResolvedValue([]);
    const { unmount } = renderHook(() =>
      useRunsPendingApprovalSet([makeRun({ status: 'running' })])
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    unmount();

    fetchPendingApprovals.mockClear();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(fetchPendingApprovals).not.toHaveBeenCalled();
  });
});

describe('resolveDisplayStatus', () => {
  it('overrides to pending_approval when running and the run id is in the pending set', () => {
    const run = makeRun({ status: 'running' });
    const pendingRunIds = new Set(['run-1']);
    expect(resolveDisplayStatus(run, pendingRunIds)).toBe('pending_approval');
  });

  it('leaves the status untouched when running but not in the pending set', () => {
    const run = makeRun({ status: 'running' });
    expect(resolveDisplayStatus(run, new Set())).toBe('running');
  });

  it('leaves non-running statuses untouched even if the id is (stale) in the pending set', () => {
    const run = makeRun({ status: 'completed' });
    const pendingRunIds = new Set(['run-1']);
    expect(resolveDisplayStatus(run, pendingRunIds)).toBe('completed');
  });
});
