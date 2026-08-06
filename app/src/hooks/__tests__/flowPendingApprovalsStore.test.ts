import { act, renderHook } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PendingApproval } from '../../services/api/approvalApi';
import {
  getFlowPendingApprovalsSnapshot,
  refreshFlowPendingApprovals,
  resetFlowPendingApprovalsStoreForTests,
  retainFlowPendingApprovalsPolling,
  useFlowPendingApprovalsSource,
} from '../flowPendingApprovalsStore';

const fetchPendingApprovals = vi.hoisted(() => vi.fn());
const debugLog = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/approvalApi', () => ({ fetchPendingApprovals }));
vi.mock('debug', () => ({ default: () => debugLog }));

function makeApproval(overrides: Partial<PendingApproval> = {}): PendingApproval {
  return {
    request_id: 'req-1',
    tool_name: 'shell',
    action_summary: 'Run a private command',
    args_redacted: {},
    session_id: 'session-1',
    created_at: '2026-01-01T00:00:00Z',
    expires_at: null,
    source_context: { kind: 'flow', flow_id: 'flow-1', run_id: 'run-1' },
    ...overrides,
  };
}

describe('flowPendingApprovalsStore', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    resetFlowPendingApprovalsStoreForTests();
  });

  afterEach(() => {
    resetFlowPendingApprovalsStoreForTests();
    vi.useRealTimers();
  });

  it('shares one immediate request and one timer across concurrent enabled consumers', async () => {
    fetchPendingApprovals.mockResolvedValue([makeApproval()]);

    const first = renderHook(() => useFlowPendingApprovalsSource(true));
    const second = renderHook(() => useFlowPendingApprovalsSource(true));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);
    expect(first.result.current.polling).toBe(true);
    expect(second.result.current.approvals).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(1);

    first.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(2);

    second.unmount();
    expect(getFlowPendingApprovalsSnapshot().polling).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('shares a deferred request across the StrictMode retain cycle and cleans up safely', async () => {
    let resolveRequest!: (approvals: PendingApproval[]) => void;
    let activeRequests = 0;
    let maxActiveRequests = 0;
    fetchPendingApprovals.mockImplementation(
      () =>
        new Promise<PendingApproval[]>(resolve => {
          activeRequests += 1;
          maxActiveRequests = Math.max(maxActiveRequests, activeRequests);
          resolveRequest = approvals => {
            activeRequests -= 1;
            resolve(approvals);
          };
        })
    );

    const source = renderHook(() => useFlowPendingApprovalsSource(true), { wrapper: StrictMode });

    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);
    expect(maxActiveRequests).toBe(1);

    await act(async () => {
      resolveRequest([makeApproval()]);
      await Promise.resolve();
    });

    expect(source.result.current.approvals).toHaveLength(1);
    expect(source.result.current.polling).toBe(true);
    expect(vi.getTimerCount()).toBe(1);

    source.unmount();
    expect(getFlowPendingApprovalsSnapshot().polling).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('isolates a disabled source from live snapshots and reconnects when enabled', async () => {
    fetchPendingApprovals.mockResolvedValueOnce([
      makeApproval({ request_id: 'req-seed', action_summary: 'Seed approval' }),
    ]);
    await refreshFlowPendingApprovals();

    let renderCount = 0;
    const source = renderHook(
      ({ enabled }: { enabled: boolean }) => {
        renderCount += 1;
        return useFlowPendingApprovalsSource(enabled);
      },
      { initialProps: { enabled: false } }
    );
    const disabledSnapshot = source.result.current;
    expect(disabledSnapshot).toEqual({ approvals: [], error: null, polling: false });
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    const disabledRenderCount = renderCount;
    fetchPendingApprovals.mockResolvedValueOnce([
      makeApproval({ request_id: 'req-live', action_summary: 'Live approval' }),
    ]);
    await act(async () => {
      await refreshFlowPendingApprovals();
    });

    expect(renderCount).toBe(disabledRenderCount);
    expect(source.result.current).toBe(disabledSnapshot);

    let resolveEnabledRefresh!: (approvals: PendingApproval[]) => void;
    fetchPendingApprovals.mockReturnValueOnce(
      new Promise<PendingApproval[]>(resolve => {
        resolveEnabledRefresh = resolve;
      })
    );
    source.rerender({ enabled: true });
    expect(source.result.current.approvals[0]?.request_id).toBe('req-live');
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(3);

    await act(async () => {
      resolveEnabledRefresh([makeApproval({ request_id: 'req-enabled' })]);
      await Promise.resolve();
    });
    expect(source.result.current).toMatchObject({
      approvals: [expect.objectContaining({ request_id: 'req-enabled' })],
      polling: true,
    });

    source.rerender({ enabled: false });
    expect(source.result.current).toBe(disabledSnapshot);
    expect(getFlowPendingApprovalsSnapshot()).toMatchObject({
      approvals: [expect.objectContaining({ request_id: 'req-enabled' })],
      polling: false,
    });
    expect(vi.getTimerCount()).toBe(0);
  });

  it('deeply clones and freezes fetched approvals without mutating API-owned objects', async () => {
    const sourceContext = {
      kind: 'flow' as const,
      flow_id: 'flow-1',
      run_id: 'run-1',
      node_id: 'node-1',
    };
    const argsRedacted = { command: { length: 42 }, flags: ['safe'] };
    const fetched = [makeApproval({ source_context: sourceContext, args_redacted: argsRedacted })];
    fetchPendingApprovals.mockResolvedValue(fetched);

    const initial = getFlowPendingApprovalsSnapshot();
    expect(getFlowPendingApprovalsSnapshot()).toBe(initial);
    expect(Object.isFrozen(initial)).toBe(true);
    expect(Object.isFrozen(initial.approvals)).toBe(true);

    await refreshFlowPendingApprovals();
    const successful = getFlowPendingApprovalsSnapshot();
    expect(successful).toBe(getFlowPendingApprovalsSnapshot());
    expect(successful.approvals).not.toBe(fetched);
    expect(successful.approvals[0]).not.toBe(fetched[0]);
    expect(successful.approvals[0].source_context).not.toBe(sourceContext);
    expect(successful.approvals[0].args_redacted).not.toBe(argsRedacted);
    expect(Object.isFrozen(successful)).toBe(true);
    expect(Object.isFrozen(successful.approvals)).toBe(true);
    expect(Object.isFrozen(successful.approvals[0])).toBe(true);
    expect(Object.isFrozen(successful.approvals[0].source_context)).toBe(true);
    expect(Object.isFrozen(successful.approvals[0].args_redacted)).toBe(true);
    expect(
      Object.isFrozen((successful.approvals[0].args_redacted as typeof argsRedacted).command)
    ).toBe(true);
    expect(
      Object.isFrozen((successful.approvals[0].args_redacted as typeof argsRedacted).flags)
    ).toBe(true);

    expect(() => {
      successful.approvals[0].action_summary = 'consumer mutation';
    }).toThrow();
    expect(() => {
      (successful.approvals[0].source_context as typeof sourceContext).run_id = 'changed';
    }).toThrow();
    expect(() => {
      (successful.approvals[0].args_redacted as typeof argsRedacted).command.length = 0;
    }).toThrow();

    expect(fetched[0].action_summary).toBe('Run a private command');
    expect(sourceContext.run_id).toBe('run-1');
    expect(argsRedacted.command.length).toBe(42);
    expect(Object.isFrozen(fetched[0])).toBe(false);
    expect(Object.isFrozen(sourceContext)).toBe(false);
    expect(Object.isFrozen(argsRedacted)).toBe(false);
    expect(getFlowPendingApprovalsSnapshot()).toBe(successful);
  });

  it.each([null, undefined, {}, { reason: 'opaque failure' }])(
    'uses the safe fallback for an unusable rejection value: %j',
    async rejection => {
      fetchPendingApprovals.mockRejectedValue(rejection);

      await refreshFlowPendingApprovals();

      expect(getFlowPendingApprovalsSnapshot().error).toBe('Unable to load pending approvals');
    }
  );

  it('keeps the last good approvals, normalizes an error, and retries on the next tick', async () => {
    fetchPendingApprovals
      .mockResolvedValueOnce([makeApproval()])
      .mockRejectedValueOnce(new Error('temporary transport failure'))
      .mockResolvedValueOnce([]);
    const release = retainFlowPendingApprovalsPolling();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getFlowPendingApprovalsSnapshot().approvals).toHaveLength(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(getFlowPendingApprovalsSnapshot()).toMatchObject({
      approvals: [expect.objectContaining({ request_id: 'req-1' })],
      error: 'temporary transport failure',
      polling: true,
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(getFlowPendingApprovalsSnapshot()).toMatchObject({
      approvals: [],
      error: null,
      polling: true,
    });
    release();
  });

  it('suppresses an in-flight request publish and timer after final release', async () => {
    let resolveRequest!: (approvals: PendingApproval[]) => void;
    fetchPendingApprovals.mockReturnValue(
      new Promise<PendingApproval[]>(resolve => {
        resolveRequest = resolve;
      })
    );
    const release = retainFlowPendingApprovalsPolling();
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    release();
    resolveRequest([makeApproval()]);
    await act(async () => {
      await Promise.resolve();
    });

    expect(getFlowPendingApprovalsSnapshot()).toMatchObject({ approvals: [], polling: false });
    expect(vi.getTimerCount()).toBe(0);
  });

  it('cancels a queued retained refresh when the final consumer releases', async () => {
    let resolveActiveRequest!: (approvals: PendingApproval[]) => void;
    fetchPendingApprovals
      .mockImplementationOnce(
        () =>
          new Promise<PendingApproval[]>(resolve => {
            resolveActiveRequest = resolve;
          })
      )
      .mockResolvedValueOnce([]);
    const release = retainFlowPendingApprovalsPolling();
    const queuedRefresh = refreshFlowPendingApprovals();
    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);

    release();
    const releasedSnapshot = getFlowPendingApprovalsSnapshot();

    await act(async () => {
      resolveActiveRequest([makeApproval()]);
      await queuedRefresh;
    });

    expect(fetchPendingApprovals).toHaveBeenCalledTimes(1);
    expect(getFlowPendingApprovalsSnapshot()).toBe(releasedSnapshot);
    expect(getFlowPendingApprovalsSnapshot()).toMatchObject({ approvals: [], polling: false });
    expect(vi.getTimerCount()).toBe(0);
  });

  it('logs only safe failure metadata, without approval payloads or error text', async () => {
    fetchPendingApprovals
      .mockResolvedValueOnce([makeApproval({ action_summary: 'private user-authored text' })])
      .mockRejectedValueOnce(new Error('private transport error'));
    const release = retainFlowPendingApprovalsPolling();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });

    const logged = JSON.stringify(debugLog.mock.calls);
    expect(logged).not.toContain('private user-authored text');
    expect(logged).not.toContain('private transport error');
    release();
  });
});
