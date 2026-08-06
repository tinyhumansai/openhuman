import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import { useFlowRunsQuery } from '../useFlowRunsQuery';

const listFlowRuns = vi.hoisted(() => vi.fn());
const listAllFlowRuns = vi.hoisted(() => vi.fn());
const stateUpdate = vi.hoisted(() => vi.fn());
const debugMock = vi.hoisted(() => {
  const namespaces: string[] = [];
  const log = vi.fn();
  return {
    namespaces,
    log,
    factory: (namespace: string) => {
      namespaces.push(namespace);
      return log;
    },
  };
});

vi.mock('../../services/api/flowsApi', () => ({ listFlowRuns, listAllFlowRuns }));
vi.mock('debug', () => ({ default: debugMock.factory }));
vi.mock('react', async importOriginal => {
  const react = await importOriginal<typeof import('react')>();
  return {
    ...react,
    useState: ((initialState: unknown) => {
      const [state, updateState] = react.useState(initialState);
      return [
        state,
        (nextState: unknown) => {
          stateUpdate(nextState);
          updateState(nextState);
        },
      ];
    }) as typeof react.useState,
  };
});

function makeRun(id: string, flowId = 'flow-1'): FlowRun {
  return {
    id,
    flow_id: flowId,
    thread_id: id,
    status: 'completed',
    started_at: '2026-07-24T00:00:00Z',
    steps: [],
    pending_approvals: [],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('useFlowRunsQuery', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('loads a flow scope with the flow endpoint and exposes the initial loading state', async () => {
    const request = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValue(request.promise);

    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );

    await waitFor(() => expect(result.current.loading).toBe(true));
    expect(result.current.runs).toEqual([]);
    expect(listFlowRuns).toHaveBeenCalledWith('flow-1');
    expect(listAllFlowRuns).not.toHaveBeenCalled();

    await act(async () => {
      request.resolve([makeRun('run-1')]);
      await request.promise;
    });

    expect(result.current).toMatchObject({ runs: [makeRun('run-1')], loading: false, error: null });
  });

  it('uses the aggregate endpoint for an all scope', async () => {
    listAllFlowRuns.mockResolvedValue([makeRun('run-all')]);

    const { result } = renderHook(() => useFlowRunsQuery({ scope: { kind: 'all' } }));

    await waitFor(() => expect(result.current.runs).toEqual([makeRun('run-all')]));
    expect(listAllFlowRuns).toHaveBeenCalledTimes(1);
    expect(listFlowRuns).not.toHaveBeenCalled();
  });

  it("does not refetch when only the caller's scope object identity changes", async () => {
    listFlowRuns.mockResolvedValue([makeRun('run-1')]);
    const { result, rerender } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );
    await waitFor(() => expect(result.current.runs).toEqual([makeRun('run-1')]));

    rerender();

    expect(listFlowRuns).toHaveBeenCalledTimes(1);
  });

  it('resets without fetching when disabled or when the flow id is null', async () => {
    listFlowRuns.mockResolvedValue([makeRun('run-1')]);
    const { result, rerender } = renderHook(
      ({ flowId, enabled }: { flowId: string | null; enabled: boolean }) =>
        useFlowRunsQuery({ scope: { kind: 'flow', flowId }, enabled }),
      {
        initialProps: { flowId: 'flow-1', enabled: true } as {
          flowId: string | null;
          enabled: boolean;
        },
      }
    );

    await waitFor(() => expect(result.current.runs).toEqual([makeRun('run-1')]));

    rerender({ flowId: 'flow-1', enabled: false });
    expect(result.current).toMatchObject({ runs: [], loading: false, error: null });
    expect(listFlowRuns).toHaveBeenCalledTimes(1);

    rerender({ flowId: null, enabled: true });
    expect(result.current).toMatchObject({ runs: [], loading: false, error: null });
    expect(listFlowRuns).toHaveBeenCalledTimes(1);
  });

  it('resets and fetches the new flow when the flow id changes', async () => {
    const flowTwoRequest = deferred<FlowRun[]>();
    listFlowRuns.mockImplementation((flowId: string) =>
      flowId === 'flow-1' ? Promise.resolve([makeRun('run-1')]) : flowTwoRequest.promise
    );

    const { result, rerender } = renderHook(
      ({ flowId }: { flowId: string }) => useFlowRunsQuery({ scope: { kind: 'flow', flowId } }),
      { initialProps: { flowId: 'flow-1' } }
    );
    await waitFor(() => expect(result.current.runs).toEqual([makeRun('run-1')]));

    rerender({ flowId: 'flow-2' });
    await waitFor(() => expect(result.current.loading).toBe(true));
    expect(result.current.runs).toEqual([]);
    expect(listFlowRuns).toHaveBeenLastCalledWith('flow-2');

    await act(async () => {
      flowTwoRequest.resolve([makeRun('run-2', 'flow-2')]);
      await flowTwoRequest.promise;
    });
    expect(result.current.runs).toEqual([makeRun('run-2', 'flow-2')]);
  });

  it('normalizes foreground failures and clears the error on a later refresh', async () => {
    listFlowRuns.mockRejectedValueOnce({ code: 'offline' });
    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );

    await waitFor(() => expect(result.current.error).toBe('[object Object]'));
    expect(result.current.loading).toBe(false);

    const request = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValueOnce(request.promise);
    let refreshPromise!: Promise<void>;
    act(() => {
      refreshPromise = result.current.refresh();
    });
    expect(result.current.loading).toBe(true);
    expect(result.current.error).toBeNull();

    await act(async () => {
      request.resolve([makeRun('run-2')]);
      await refreshPromise;
    });
    expect(result.current).toMatchObject({ runs: [makeRun('run-2')], loading: false, error: null });
  });

  it('keeps visible data, loading, and error unchanged during a silent refresh', async () => {
    listFlowRuns.mockResolvedValueOnce([makeRun('run-1')]);
    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );
    await waitFor(() => expect(result.current.runs).toEqual([makeRun('run-1')]));

    const request = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValueOnce(request.promise);
    let refreshPromise!: Promise<void>;
    act(() => {
      refreshPromise = result.current.refreshSilently();
    });
    expect(result.current).toMatchObject({ runs: [makeRun('run-1')], loading: false, error: null });

    await act(async () => {
      request.resolve([makeRun('run-2')]);
      await refreshPromise;
    });
    expect(result.current.runs).toEqual([makeRun('run-2')]);
  });

  it('drops silent failures without exposing or logging the raw payload', async () => {
    listFlowRuns.mockResolvedValueOnce([makeRun('run-1')]);
    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );
    await waitFor(() => expect(result.current.runs).toEqual([makeRun('run-1')]));

    const sensitivePayload = Object.assign(new Error('private transport error'), {
      token: 'do-not-log-me',
    });
    listFlowRuns.mockRejectedValueOnce(sensitivePayload);
    await act(async () => {
      await result.current.refreshSilently();
    });

    expect(result.current).toMatchObject({ runs: [makeRun('run-1')], loading: false, error: null });
    expect(debugMock.namespaces).toEqual(['app:flows:runs-query']);
    expect(debugMock.log).toHaveBeenCalledExactlyOnceWith(
      'silent refresh failed: scope=%s',
      'flow'
    );
    const logged = JSON.stringify(debugMock.log.mock.calls);
    expect(logged).not.toContain('do-not-log-me');
    expect(logged).not.toContain('private transport error');
  });

  it('lets the latest silent request win over an older foreground request', async () => {
    const initial = deferred<FlowRun[]>();
    const latest = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValueOnce(initial.promise).mockReturnValueOnce(latest.promise);
    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );
    await waitFor(() => expect(result.current.loading).toBe(true));

    let silentPromise!: Promise<void>;
    act(() => {
      silentPromise = result.current.refreshSilently();
    });
    await act(async () => {
      latest.resolve([makeRun('latest')]);
      await silentPromise;
    });
    expect(result.current.runs).toEqual([makeRun('latest')]);
    expect(result.current.loading).toBe(false);

    await act(async () => {
      initial.resolve([makeRun('stale')]);
      await initial.promise;
    });
    expect(result.current.runs).toEqual([makeRun('latest')]);
    expect(result.current.loading).toBe(false);
  });

  it('keeps the foreground request active when a newer silent request fails', async () => {
    const initial = deferred<FlowRun[]>();
    const latest = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValueOnce(initial.promise).mockReturnValueOnce(latest.promise);
    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );
    await waitFor(() => expect(result.current.loading).toBe(true));

    let silentPromise!: Promise<void>;
    act(() => {
      silentPromise = result.current.refreshSilently();
    });
    await act(async () => {
      latest.reject(new Error('offline'));
      await silentPromise;
    });
    expect(result.current).toMatchObject({ runs: [], loading: true, error: null });

    await act(async () => {
      initial.resolve([makeRun('foreground')]);
      await initial.promise;
    });
    expect(result.current).toMatchObject({
      runs: [makeRun('foreground')],
      loading: false,
      error: null,
    });
  });

  it('lets the latest foreground request win over an older silent request', async () => {
    listFlowRuns.mockResolvedValueOnce([makeRun('initial')]);
    const { result } = renderHook(() =>
      useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } })
    );
    await waitFor(() => expect(result.current.runs).toEqual([makeRun('initial')]));

    const silent = deferred<FlowRun[]>();
    const foreground = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValueOnce(silent.promise).mockReturnValueOnce(foreground.promise);
    let silentPromise!: Promise<void>;
    act(() => {
      silentPromise = result.current.refreshSilently();
    });
    let foregroundPromise!: Promise<void>;
    act(() => {
      foregroundPromise = result.current.refresh();
    });

    await act(async () => {
      foreground.resolve([makeRun('latest')]);
      await foregroundPromise;
    });
    expect(result.current).toMatchObject({
      runs: [makeRun('latest')],
      loading: false,
      error: null,
    });

    await act(async () => {
      silent.resolve([makeRun('stale')]);
      await silentPromise;
    });
    expect(result.current.runs).toEqual([makeRun('latest')]);
  });

  it.each(['success', 'failure'] as const)(
    'does not notify state or render when deferred %s settles after unmount',
    async outcome => {
      const request = deferred<FlowRun[]>();
      listFlowRuns.mockReturnValue(request.promise);
      const renderTransition = vi.fn();
      const { result, unmount } = renderHook(() => {
        const current = useFlowRunsQuery({ scope: { kind: 'flow', flowId: 'flow-1' } });
        renderTransition(current);
        return current;
      });
      await waitFor(() => expect(result.current.loading).toBe(true));

      const stateUpdateCount = stateUpdate.mock.calls.length;
      const renderCount = renderTransition.mock.calls.length;
      unmount();
      await act(async () => {
        if (outcome === 'success') {
          request.resolve([makeRun('late')]);
        } else {
          request.reject(new Error('late failure'));
        }
        await request.promise.catch(() => undefined);
        await Promise.resolve();
      });

      expect(stateUpdate).toHaveBeenCalledTimes(stateUpdateCount);
      expect(renderTransition).toHaveBeenCalledTimes(renderCount);
    }
  );
});
