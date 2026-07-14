/**
 * FlowRunsSidebar — the flow canvas's projected run-history sidebar. Asserts
 * the run list renders, a run row opens the {@link FlowRunInspectorDrawer},
 * and (issue B22) the drawer's "Fix with agent" action navigates to this same
 * flow's canvas seeded with a `copilotRepair` state and closes the sidebar's
 * own drawer — this sidebar is only ever mounted while the user is ALREADY on
 * the failing run's own `/flows/:id` canvas (`FlowCanvasPage` projects it into
 * the shell sidebar), so re-navigating to the SAME route with a fresh repair
 * seed is the fix (see `FlowCanvasPage.tsx`'s `locationKey`-based copilot
 * panel remount, which reacts to exactly this navigation).
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import { store } from '../../store';
import FlowRunsSidebar from './FlowRunsSidebar';

const listFlowRuns = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ listFlowRuns }));

// Capture the props handed to the drawer so "Fix with agent" can be invoked
// directly without standing up the drawer's own run-polling machinery
// (mirrors `FlowApprovalCard.test.tsx`'s stub pattern).
const inspectorDrawerProps = vi.hoisted(() => ({
  current: null as Record<string, unknown> | null,
}));
vi.mock('./FlowRunInspectorDrawer', () => ({
  FLOW_RUN_STATUS_ACCENT: {
    running: '',
    completed: '',
    completed_with_warnings: '',
    pending_approval: '',
    failed: '',
    cancelled: '',
  },
  FLOW_RUN_STATUS_DOT: {
    running: '',
    completed: '',
    completed_with_warnings: '',
    pending_approval: '',
    failed: '',
    cancelled: '',
  },
  FLOW_RUN_STATUS_KEY: {
    running: 'flowRuns.status.running',
    completed: 'flowRuns.status.completed',
    completed_with_warnings: 'flowRuns.status.completed_with_warnings',
    pending_approval: 'flowRuns.status.pending_approval',
    failed: 'flowRuns.status.failed',
    cancelled: 'flowRuns.status.cancelled',
  },
  FlowRunInspectorDrawer: (props: Record<string, unknown>) => {
    inspectorDrawerProps.current = props;
    return props.runId ? (
      <div data-testid="flow-run-inspector-drawer-stub">{props.runId as string}</div>
    ) : null;
  },
}));

function makeRun(overrides: Partial<FlowRun> = {}): FlowRun {
  return {
    id: 'run-1',
    flow_id: 'flow-1',
    thread_id: 'run-1',
    status: 'failed',
    started_at: '2026-07-13T18:23:00Z',
    finished_at: '2026-07-13T18:23:05Z',
    steps: [],
    pending_approvals: [],
    error: 'GMAIL_SEND_EMAIL: empty body',
    ...overrides,
  };
}

/** Renders whatever `location.state` a navigation landed with, for assertions. */
function LocationStateProbe() {
  const location = useLocation();
  return <div data-testid="location-state-probe">{JSON.stringify(location.state)}</div>;
}

function renderSidebar(flowId = 'flow-1') {
  return render(
    <Provider store={store}>
      <MemoryRouter initialEntries={[`/flows/${flowId}`]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowRunsSidebar flowId={flowId} />} />
        </Routes>
      </MemoryRouter>
    </Provider>
  );
}

describe('FlowRunsSidebar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    inspectorDrawerProps.current = null;
  });

  it('lists runs and opens the inspector drawer for the clicked run', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    renderSidebar();

    const row = await screen.findByTestId('flow-runs-sidebar-run-run-1');
    expect(screen.queryByTestId('flow-run-inspector-drawer-stub')).not.toBeInTheDocument();

    fireEvent.click(row);

    expect(screen.getByTestId('flow-run-inspector-drawer-stub')).toHaveTextContent('run-1');
  });

  it('passes onFixWithAgent through to the run inspector drawer', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    renderSidebar();

    fireEvent.click(await screen.findByTestId('flow-runs-sidebar-run-run-1'));

    expect(inspectorDrawerProps.current?.onFixWithAgent).toBeInstanceOf(Function);
  });

  it('"Fix with agent" closes the drawer and navigates to the same flow seeded with the repair context (B22)', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/flows/flow-1']}>
          <Routes>
            <Route
              path="/flows/:id"
              element={
                <>
                  <FlowRunsSidebar flowId="flow-1" />
                  <LocationStateProbe />
                </>
              }
            />
          </Routes>
        </MemoryRouter>
      </Provider>
    );

    fireEvent.click(await screen.findByTestId('flow-runs-sidebar-run-run-1'));
    expect(screen.getByTestId('flow-run-inspector-drawer-stub')).toBeInTheDocument();

    act(() => {
      (
        inspectorDrawerProps.current?.onFixWithAgent as (request: {
          flowId: string;
          runId: string;
          error?: string | null;
          failingNodeIds?: string[];
        }) => void
      )({
        flowId: 'flow-1',
        runId: 'run-1',
        error: 'GMAIL_SEND_EMAIL: empty body',
        failingNodeIds: ['send_summary'],
      });
    });

    // The sidebar's own run-inspector drawer closes (repair takes over).
    await waitFor(() =>
      expect(screen.queryByTestId('flow-run-inspector-drawer-stub')).not.toBeInTheDocument()
    );
    const probe = screen.getByTestId('location-state-probe');
    expect(JSON.parse(probe.textContent ?? 'null')).toEqual({
      copilotRepair: {
        runId: 'run-1',
        error: 'GMAIL_SEND_EMAIL: empty body',
        failingNodeIds: ['send_summary'],
      },
    });
  });

  it('shows the empty state when there are no runs', async () => {
    listFlowRuns.mockResolvedValue([]);
    renderSidebar();

    expect(await screen.findByTestId('flow-runs-sidebar-empty')).toBeInTheDocument();
  });
});
