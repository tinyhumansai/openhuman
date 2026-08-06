/**
 * Approve/Dismiss/View-run/Fix-with-agent contract for the
 * flow-pending-approval notification card (issues B3a + B3b + B22). Asserts
 * that Approve reads `{ flow_id, thread_id, node_ids }` from the notification's
 * action payload, calls `flowsApi.resumeFlow` with those args, clears the
 * notification on success, surfaces a localized error on failure (including
 * when `node_ids` contains non-string entries — an invalid payload), that
 * Dismiss clears the notification WITHOUT calling any RPC (there is no
 * `flows_deny` endpoint yet), that "View run" opens the
 * {@link FlowRunInspectorDrawer}, and that the drawer's "Fix with agent"
 * action navigates to the flow's canvas seeded with a `copilotRepair` state
 * (issue B22 — this card can render anywhere in the app, so it always
 * navigates rather than assuming the canvas is already open).
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { type NotificationItem } from '../../store/notificationSlice';
import FlowApprovalCard from './FlowApprovalCard';

const resumeFlow = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ resumeFlow }));

// Capture the props `FlowApprovalCard` hands the drawer (mirrors
// `FlowCanvasPage.test.tsx`'s copilot-panel stub pattern) so tests can invoke
// `onFixWithAgent` directly without needing the drawer's own run-polling
// machinery.
const inspectorDrawerProps = vi.hoisted(() => ({
  current: null as Record<string, unknown> | null,
}));
vi.mock('../flows/FlowRunInspectorDrawer', () => ({
  FlowRunInspectorDrawer: (props: Record<string, unknown>) => {
    inspectorDrawerProps.current = props;
    return props.runId ? (
      <div data-testid="flow-run-inspector-drawer-stub">{props.runId as string}</div>
    ) : null;
  },
}));

function makeItem(overrides: Partial<NotificationItem> = {}): NotificationItem {
  return {
    id: 'flow-pending-approval:flow-1:thread-1',
    category: 'agents',
    title: 'Workflow needs approval',
    body: '"Deploy pipeline" is waiting on 2 approvals before it can continue.',
    timestamp: Date.now(),
    read: false,
    actions: [
      {
        actionId: 'approve',
        label: 'Review',
        payload: { flow_id: 'flow-1', thread_id: 'thread-1', node_ids: ['node-a', 'node-b'] },
      },
    ],
    ...overrides,
  };
}

/** Renders whatever `location.state` a navigation landed with, for assertions. */
function LocationStateProbe() {
  const location = useLocation();
  return <div data-testid="location-state-probe">{JSON.stringify(location.state)}</div>;
}

function renderCard(item: NotificationItem) {
  return render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/home']}>
        <Routes>
          <Route path="/home" element={<FlowApprovalCard notification={item} />} />
          <Route path="/flows/:id" element={<LocationStateProbe />} />
        </Routes>
      </MemoryRouter>
    </Provider>
  );
}

describe('FlowApprovalCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch({ type: 'notifications/clearAll' });
    inspectorDrawerProps.current = null;
  });

  it('renders both Approve and Dismiss buttons', () => {
    renderCard(makeItem());
    expect(screen.getByTestId('flow-approval-approve')).toBeInTheDocument();
    expect(screen.getByTestId('flow-approval-dismiss')).toBeInTheDocument();
  });

  it('renders as an alertdialog with the notification body', () => {
    renderCard(makeItem());
    const card = screen.getByTestId('flow-approval-card');
    expect(card).toHaveAttribute('role', 'alertdialog');
    expect(screen.getByText(makeItem().body)).toBeInTheDocument();
  });

  it('calls resumeFlow with flow_id/thread_id/node_ids extracted from the action payload', async () => {
    resumeFlow.mockResolvedValue({ output: null, pending_approvals: [], thread_id: 'thread-1' });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('flow-approval-approve'));

    await waitFor(() => expect(resumeFlow).toHaveBeenCalledTimes(1));
    expect(resumeFlow).toHaveBeenCalledWith('flow-1', 'thread-1', ['node-a', 'node-b']);
  });

  it('marks the notification read and clears its actions on a successful approve', async () => {
    resumeFlow.mockResolvedValue({ output: null, pending_approvals: [], thread_id: 'thread-1' });
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('flow-approval-approve'));

    await waitFor(() => {
      const item = store
        .getState()
        .notifications.items.find(i => i.id === 'flow-pending-approval:flow-1:thread-1');
      expect(item?.read).toBe(true);
      expect(item?.actions ?? []).toHaveLength(0);
    });
  });

  it('does NOT clear the notification when the run parks again on the next gate', async () => {
    // Sequential gates: resume returns with pending_approvals still non-empty and
    // the core re-publishes the same-id prompt — the card must not wipe it.
    resumeFlow.mockResolvedValue({
      output: null,
      pending_approvals: ['node-c'],
      thread_id: 'thread-1',
    });
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('flow-approval-approve'));

    await waitFor(() => expect(resumeFlow).toHaveBeenCalledTimes(1));
    const item = store
      .getState()
      .notifications.items.find(i => i.id === 'flow-pending-approval:flow-1:thread-1');
    expect(item?.actions).toHaveLength(1);
    expect(item?.read).toBe(false);
    // Approve re-enabled so the user can act on the next gate.
    await waitFor(() => expect(screen.getByTestId('flow-approval-approve')).not.toBeDisabled());
  });

  it('shows a localized error and re-enables the buttons when resumeFlow rejects', async () => {
    resumeFlow.mockRejectedValue(new Error('no pending approval matches'));
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('flow-approval-approve'));

    await waitFor(() => {
      expect(screen.getByTestId('flow-approval-approve')).not.toBeDisabled();
    });
    expect(
      screen.getByText(
        (_content, element) =>
          element?.tagName.toLowerCase() === 'p' &&
          (element?.textContent ?? '').includes('Could not resume the workflow. Please try again.')
      )
    ).toBeInTheDocument();
    // The notification must NOT have been cleared on failure.
    const item = store
      .getState()
      .notifications.items.find(i => i.id === 'flow-pending-approval:flow-1:thread-1');
    expect(item?.actions).toHaveLength(1);
  });

  it('disables both buttons while the approve RPC is in flight', async () => {
    let resolve!: (v: unknown) => void;
    resumeFlow.mockImplementation(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('flow-approval-approve'));

    expect(screen.getByTestId('flow-approval-approve')).toBeDisabled();
    expect(screen.getByTestId('flow-approval-dismiss')).toBeDisabled();

    resolve({ output: null, pending_approvals: [], thread_id: 'thread-1' });
    await waitFor(() => expect(screen.getByTestId('flow-approval-approve')).not.toBeDisabled());
  });

  it('dismiss clears the notification without calling resumeFlow', async () => {
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('flow-approval-dismiss'));

    await waitFor(() => {
      const item = store
        .getState()
        .notifications.items.find(i => i.id === 'flow-pending-approval:flow-1:thread-1');
      expect(item?.read).toBe(true);
      expect(item?.actions ?? []).toHaveLength(0);
    });
    expect(resumeFlow).not.toHaveBeenCalled();
  });

  it('treats non-string node_ids as an invalid payload (Approve errors, no resumeFlow call)', async () => {
    renderCard(
      makeItem({
        actions: [
          {
            actionId: 'approve',
            label: 'Review',
            payload: { flow_id: 'flow-1', thread_id: 'thread-1', node_ids: [42, null] },
          },
        ],
      })
    );

    fireEvent.click(screen.getByTestId('flow-approval-approve'));

    await waitFor(() => {
      expect(
        screen.getByText(
          (_content, element) =>
            element?.tagName.toLowerCase() === 'p' &&
            (element?.textContent ?? '').includes(
              'Could not resume the workflow. Please try again.'
            )
        )
      ).toBeInTheDocument();
    });
    expect(resumeFlow).not.toHaveBeenCalled();
  });

  it('does not render "View run" when the payload is invalid', () => {
    renderCard(
      makeItem({
        actions: [{ actionId: 'approve', label: 'Review', payload: { flow_id: 'flow-1' } }],
      })
    );
    expect(screen.queryByTestId('flow-approval-view-run')).not.toBeInTheDocument();
  });

  it('"View run" opens the run inspector drawer for the payload thread_id', () => {
    renderCard(makeItem());

    expect(screen.queryByTestId('flow-run-inspector-drawer-stub')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('flow-approval-view-run'));

    const drawer = screen.getByTestId('flow-run-inspector-drawer-stub');
    expect(drawer).toBeInTheDocument();
    expect(drawer).toHaveTextContent('thread-1');
  });

  it('passes onFixWithAgent through to the run inspector drawer', () => {
    renderCard(makeItem());
    fireEvent.click(screen.getByTestId('flow-approval-view-run'));

    expect(inspectorDrawerProps.current?.onFixWithAgent).toBeInstanceOf(Function);
  });

  it('"Fix with agent" navigates to the flow canvas seeded with the repair context (B22)', async () => {
    renderCard(makeItem());
    fireEvent.click(screen.getByTestId('flow-approval-view-run'));
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
        runId: 'thread-1',
        error: 'GMAIL_SEND_EMAIL: empty body',
        failingNodeIds: ['send_summary'],
      });
    });

    // Navigated away — the drawer stub (and the whole card) unmounts.
    await waitFor(() =>
      expect(screen.queryByTestId('flow-run-inspector-drawer-stub')).not.toBeInTheDocument()
    );
    const probe = await screen.findByTestId('location-state-probe');
    expect(JSON.parse(probe.textContent ?? 'null')).toEqual({
      copilotRepair: {
        runId: 'thread-1',
        error: 'GMAIL_SEND_EMAIL: empty body',
        failingNodeIds: ['send_summary'],
      },
    });
  });
});
