/**
 * FlowRunsDrawer (issue B5a.1) — rendering contract.
 *
 * Asserts: renders null when `flowId` is null; loading state; renders fetched
 * runs (status pill, started-at, truncated id); empty state; error state;
 * clicking a run opens `FlowRunInspectorDrawer` for it, stacked on top;
 * closing the inspector returns to the runs list; Escape/backdrop/✕ close the
 * runs drawer itself.
 *
 * Mocks `flowsApi.listFlowRuns` (the one-shot fetch this drawer owns) and
 * `FlowRunInspectorDrawer` (its own behavior is covered by
 * `FlowRunInspectorDrawer.test.tsx`) so this suite only exercises the
 * run-history list + the nesting contract between the two drawers.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import { store } from '../../store';
import FlowRunsDrawer from './FlowRunsDrawer';

const listFlowRuns = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ listFlowRuns, listAllFlowRuns: vi.fn() }));

const runStarted = vi.hoisted(() => ({ callback: undefined as (() => void) | undefined }));
vi.mock('../../hooks/useFlowRunStarted', () => ({
  useFlowRunStarted: (callback: () => void) => {
    runStarted.callback = callback;
  },
}));

const runFinished = vi.hoisted(() => ({ callback: undefined as (() => void) | undefined }));
vi.mock('../../hooks/useFlowRunFinished', () => ({
  useFlowRunFinished: (callback: () => void) => {
    runFinished.callback = callback;
  },
}));

const fetchPendingApprovals = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/approvalApi', () => ({ fetchPendingApprovals }));

const FlowRunInspectorDrawer = vi.hoisted(() => vi.fn());
vi.mock('./FlowRunInspectorDrawer', () => ({
  FlowRunInspectorDrawer: (props: { runId: string | null; onClose: () => void }) => {
    FlowRunInspectorDrawer(props);
    if (!props.runId) return null;
    return (
      <div data-testid="mock-inspector">
        <span>{props.runId}</span>
        <button type="button" data-testid="mock-inspector-close" onClick={props.onClose}>
          close inspector
        </button>
      </div>
    );
  },
}));

function makeRun(overrides: Partial<FlowRun> = {}): FlowRun {
  return {
    id: 'run-1',
    flow_id: 'flow-1',
    thread_id: 'run-1',
    status: 'completed',
    started_at: '2026-01-01T00:00:00Z',
    steps: [],
    pending_approvals: [],
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(resolvePromise => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function renderDrawer(flowId: string | null, onClose: () => void, flowName?: string) {
  return render(
    <Provider store={store}>
      <FlowRunsDrawer flowId={flowId} flowName={flowName} onClose={onClose} />
    </Provider>
  );
}

describe('FlowRunsDrawer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    runStarted.callback = undefined;
    runFinished.callback = undefined;
    fetchPendingApprovals.mockResolvedValue([]);
  });

  it('refreshes immediately when a run finishes', async () => {
    listFlowRuns
      .mockResolvedValueOnce([makeRun({ status: 'running' })])
      .mockResolvedValueOnce([makeRun({ status: 'completed' })]);
    renderDrawer('flow-1', vi.fn());

    await screen.findByTestId('flow-runs-list');
    expect(runFinished.callback).toBeTypeOf('function');
    act(() => runFinished.callback?.());

    await waitFor(() => expect(listFlowRuns).toHaveBeenCalledTimes(2));
  });

  it('renders null when flowId is null', () => {
    const { container } = renderDrawer(null, vi.fn());
    expect(container).toBeEmptyDOMElement();
    expect(listFlowRuns).not.toHaveBeenCalled();
  });

  it('shows a loading state before the fetch resolves', () => {
    listFlowRuns.mockReturnValue(new Promise(() => {})); // never resolves
    renderDrawer('flow-1', vi.fn());
    const loading = screen.getByTestId('flow-runs-loading');
    expect(loading).toHaveTextContent('Loading runs…');
    expect(loading.querySelector('svg')).toBeInTheDocument();
  });

  it('retires initial loading when a run-start refresh supersedes the initial request', async () => {
    const initial = deferred<FlowRun[]>();
    const latest = deferred<FlowRun[]>();
    listFlowRuns.mockReturnValueOnce(initial.promise).mockReturnValueOnce(latest.promise);
    renderDrawer('flow-1', vi.fn());

    expect(screen.getByTestId('flow-runs-loading')).toBeInTheDocument();
    expect(runStarted.callback).toBeTypeOf('function');

    act(() => {
      runStarted.callback?.();
    });
    await act(async () => {
      latest.resolve([makeRun({ id: 'latest' })]);
      await latest.promise;
    });

    expect(screen.queryByTestId('flow-runs-loading')).not.toBeInTheDocument();
    expect(screen.getByTestId('flow-run-row-latest')).toBeInTheDocument();

    await act(async () => {
      initial.resolve([makeRun({ id: 'stale' })]);
      await initial.promise;
    });

    expect(screen.queryByTestId('flow-runs-loading')).not.toBeInTheDocument();
    expect(screen.getByTestId('flow-run-row-latest')).toBeInTheDocument();
    expect(screen.queryByTestId('flow-run-row-stale')).not.toBeInTheDocument();
  });

  it('fetches and lists runs for the flow', async () => {
    listFlowRuns.mockResolvedValue([
      makeRun({ id: 'run-1', status: 'completed' }),
      makeRun({ id: 'run-2', status: 'failed' }),
    ]);
    renderDrawer('flow-1', vi.fn(), 'Daily digest');

    expect(await screen.findByTestId('flow-runs-list')).toBeInTheDocument();
    expect(listFlowRuns).toHaveBeenCalledWith('flow-1');
    expect(screen.getByTestId('flow-run-row-run-1')).toBeInTheDocument();
    expect(screen.getByTestId('flow-run-row-run-2')).toBeInTheDocument();
    expect(screen.getByText('Runs for Daily digest')).toBeInTheDocument();
  });

  it('renders the amber pill for a run completed with warnings', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1', status: 'completed_with_warnings' })]);
    renderDrawer('flow-1', vi.fn());

    const row = await screen.findByTestId('flow-run-row-run-1');
    expect(row).toHaveTextContent('Completed with warnings');
  });

  it('falls back to a humanized label instead of "undefined" for an unrecognized status (F-m8)', async () => {
    // Run payloads are cast, never validated — a future/unknown status the
    // frontend doesn't recognize yet must not render literal "undefined".
    listFlowRuns.mockResolvedValue([
      makeRun({ id: 'run-1', status: 'archived' as FlowRun['status'] }),
    ]);
    renderDrawer('flow-1', vi.fn());

    const row = await screen.findByTestId('flow-run-row-run-1');
    expect(row).toHaveTextContent('archived');
    expect(row).not.toHaveTextContent('undefined');
  });

  it('shows "Awaiting approval" for a running run halted at a matching flow approval gate', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1', status: 'running' })]);
    fetchPendingApprovals.mockResolvedValue([
      {
        request_id: 'req-1',
        tool_name: 'SLACK_SEND_MESSAGE',
        action_summary: 'Send Slack message',
        args_redacted: {},
        session_id: 'session-1',
        created_at: '2026-01-01T00:00:00Z',
        expires_at: null,
        source_context: { kind: 'flow', flow_id: 'flow-1', run_id: 'run-1' },
      },
    ]);
    renderDrawer('flow-1', vi.fn());

    const row = await screen.findByTestId('flow-run-row-run-1');
    await waitFor(() => expect(row).toHaveTextContent('Awaiting approval'));
    expect(screen.getByTestId('flow-run-row-dot-run-1')).toHaveAttribute(
      'data-status',
      'pending_approval'
    );
  });

  it('leaves a running run without a matching flow approval labeled "Running"', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1', status: 'running' })]);
    fetchPendingApprovals.mockResolvedValue([]);
    renderDrawer('flow-1', vi.fn());

    const row = await screen.findByTestId('flow-run-row-run-1');
    await waitFor(() => expect(row).toHaveTextContent('Running'));
    expect(screen.getByTestId('flow-run-row-dot-run-1')).toHaveAttribute('data-status', 'running');
  });

  it('falls back to a generic title when no flowName is given', async () => {
    listFlowRuns.mockResolvedValue([]);
    renderDrawer('flow-1', vi.fn());
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());
    expect(screen.getByText('Workflow runs')).toBeInTheDocument();
  });

  it('shows an empty state when there are no runs', async () => {
    listFlowRuns.mockResolvedValue([]);
    renderDrawer('flow-1', vi.fn());
    expect(await screen.findByTestId('flow-runs-empty')).toHaveTextContent('No runs yet');
  });

  it('shows an error state when the fetch fails', async () => {
    listFlowRuns.mockRejectedValue(new Error('core unreachable'));
    renderDrawer('flow-1', vi.fn());
    const error = await screen.findByTestId('flow-runs-error');
    expect(error).toHaveTextContent('core unreachable');
    expect(screen.getByRole('alert')).toHaveClass('bg-coral-500/10');
  });

  it('opens the run inspector on top when a run row is clicked', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1' })]);
    renderDrawer('flow-1', vi.fn());

    const row = await screen.findByTestId('flow-run-row-run-1');
    fireEvent.click(row);

    expect(await screen.findByTestId('mock-inspector')).toHaveTextContent('run-1');
    // The runs list stays mounted underneath.
    expect(screen.getByTestId('flow-runs-list')).toBeInTheDocument();
  });

  it('returns to the run list when the inspector closes', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1' })]);
    renderDrawer('flow-1', vi.fn());

    fireEvent.click(await screen.findByTestId('flow-run-row-run-1'));
    expect(await screen.findByTestId('mock-inspector')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('mock-inspector-close'));
    expect(screen.queryByTestId('mock-inspector')).not.toBeInTheDocument();
    expect(screen.getByTestId('flow-runs-list')).toBeInTheDocument();
  });

  it('clears the selected run when the drawer changes to another flow', async () => {
    listFlowRuns.mockImplementation((flowId: string) =>
      Promise.resolve([makeRun({ id: flowId === 'flow-1' ? 'run-1' : 'run-2', flow_id: flowId })])
    );
    const { rerender } = render(
      <Provider store={store}>
        <FlowRunsDrawer flowId="flow-1" onClose={vi.fn()} />
      </Provider>
    );

    fireEvent.click(await screen.findByTestId('flow-run-row-run-1'));
    expect(screen.getByTestId('mock-inspector')).toHaveTextContent('run-1');

    rerender(
      <Provider store={store}>
        <FlowRunsDrawer flowId="flow-2" onClose={vi.fn()} />
      </Provider>
    );

    await waitFor(() => expect(screen.queryByTestId('mock-inspector')).not.toBeInTheDocument());
    expect(await screen.findByTestId('flow-run-row-run-2')).toBeInTheDocument();
  });

  it('calls onClose when the close button is clicked', async () => {
    listFlowRuns.mockResolvedValue([]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('flow-runs-close'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('keeps the backdrop as an accessible close button', async () => {
    listFlowRuns.mockResolvedValue([]);
    renderDrawer('flow-1', vi.fn());
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    expect(screen.getByTestId('flow-runs-backdrop')).toHaveAttribute('type', 'button');
    expect(screen.getByTestId('flow-runs-backdrop')).toHaveAccessibleName('Close');
  });

  it('calls onClose when a pointer lands on the backdrop', async () => {
    listFlowRuns.mockResolvedValue([]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    fireEvent.pointerDown(screen.getByTestId('flow-runs-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose exactly once for native keyboard or assistive backdrop activation', async () => {
    listFlowRuns.mockResolvedValue([]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('flow-runs-backdrop'), { detail: 0 });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not double-dismiss for a pointerdown followed by its click', async () => {
    listFlowRuns.mockResolvedValue([]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    const backdrop = screen.getByTestId('flow-runs-backdrop');
    fireEvent.pointerDown(backdrop);
    fireEvent.click(backdrop, { detail: 1 });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not close when a pointer lands inside the drawer', async () => {
    listFlowRuns.mockResolvedValue([]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    fireEvent.pointerDown(screen.getByTestId('flow-runs-empty'));
    expect(onClose).not.toHaveBeenCalled();
  });

  it('calls onClose when Escape is pressed and no run is selected', async () => {
    listFlowRuns.mockResolvedValue([]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);
    await waitFor(() => expect(screen.getByTestId('flow-runs-empty')).toBeInTheDocument());

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not close the runs drawer on Escape while the inspector is open', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1' })]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);

    fireEvent.click(await screen.findByTestId('flow-run-row-run-1'));
    expect(await screen.findByTestId('mock-inspector')).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
  });

  it('does not close the runs drawer from its backdrop while the inspector is open', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1' })]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);

    fireEvent.click(await screen.findByTestId('flow-run-row-run-1'));
    expect(await screen.findByTestId('mock-inspector')).toBeInTheDocument();

    fireEvent.pointerDown(screen.getByTestId('flow-runs-backdrop'));
    expect(onClose).not.toHaveBeenCalled();
  });

  it('does not close the runs drawer from keyboard or assistive backdrop activation while the inspector is open', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ id: 'run-1' })]);
    const onClose = vi.fn();
    renderDrawer('flow-1', onClose);

    fireEvent.click(await screen.findByTestId('flow-run-row-run-1'));
    expect(await screen.findByTestId('mock-inspector')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('flow-runs-backdrop'), { detail: 0 });
    expect(onClose).not.toHaveBeenCalled();
  });

  it('discards a stale background refetch after the drawer flips to a different flow (race guard)', async () => {
    // Regression test for a codex review finding on this PR: the live-refresh
    // `refetch` in FlowRunsDrawer didn't guard against a response landing
    // after the drawer had already switched to a different flowId, so a slow
    // flow-A refetch could clobber flow-B's already-rendered runs.
    vi.useFakeTimers();
    try {
      let flowACalls = 0;
      let resolveStaleA: ((runs: FlowRun[]) => void) | undefined;
      listFlowRuns.mockImplementation((flowId: string) => {
        if (flowId === 'flow-a') {
          flowACalls += 1;
          if (flowACalls === 1) {
            // Initial load: one active run so the live-refresh poll subscribes.
            return Promise.resolve([
              makeRun({ id: 'run-a', flow_id: 'flow-a', status: 'running' }),
            ]);
          }
          // The poll-triggered refetch — stays pending until resolved below,
          // simulating a slow response that outlives the flow switch.
          return new Promise<FlowRun[]>(resolve => {
            resolveStaleA = resolve;
          });
        }
        if (flowId === 'flow-b') {
          return Promise.resolve([
            makeRun({ id: 'run-b', flow_id: 'flow-b', status: 'completed' }),
          ]);
        }
        return Promise.resolve([]);
      });

      const { rerender } = render(
        <Provider store={store}>
          <FlowRunsDrawer flowId="flow-a" onClose={vi.fn()} />
        </Provider>
      );

      // Flush the initial load's already-resolved promise.
      await act(async () => {
        await Promise.resolve();
      });
      expect(screen.getByTestId('flow-run-row-run-a')).toBeInTheDocument();

      // Trigger the live-refresh poll fallback — issues the second, hanging
      // listFlowRuns('flow-a') call.
      await act(async () => {
        vi.advanceTimersByTime(30_000);
      });
      expect(flowACalls).toBe(2);

      // Flip the drawer to a different flow while that refetch is still in flight.
      rerender(
        <Provider store={store}>
          <FlowRunsDrawer flowId="flow-b" onClose={vi.fn()} />
        </Provider>
      );
      await act(async () => {
        await Promise.resolve();
      });
      expect(screen.getByTestId('flow-run-row-run-b')).toBeInTheDocument();

      // Now let the stale flow-a response land.
      await act(async () => {
        resolveStaleA?.([makeRun({ id: 'run-a-late', flow_id: 'flow-a', status: 'completed' })]);
        await Promise.resolve();
      });

      // Flow-b's runs must be unaffected by the late flow-a response.
      expect(screen.getByTestId('flow-run-row-run-b')).toBeInTheDocument();
      expect(screen.queryByTestId('flow-run-row-run-a-late')).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
