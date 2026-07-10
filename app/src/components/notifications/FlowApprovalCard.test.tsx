/**
 * Approve/Dismiss/View-run contract for the flow-pending-approval
 * notification card (issues B3a + B3b). Asserts that Approve reads
 * `{ flow_id, thread_id, node_ids }` from the notification's action payload,
 * calls `flowsApi.resumeFlow` with those args, clears the notification on
 * success, surfaces a localized error on failure (including when `node_ids`
 * contains non-string entries — an invalid payload), that Dismiss clears the
 * notification WITHOUT calling any RPC (there is no `flows_deny` endpoint
 * yet), and that "View run" opens the {@link FlowRunInspectorDrawer}.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { type NotificationItem } from '../../store/notificationSlice';
import FlowApprovalCard from './FlowApprovalCard';

const resumeFlow = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ resumeFlow }));

vi.mock('../flows/FlowRunInspectorDrawer', () => ({
  FlowRunInspectorDrawer: ({ runId }: { runId: string | null; onClose: () => void }) =>
    runId ? <div data-testid="flow-run-inspector-drawer-stub">{runId}</div> : null,
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

function renderCard(item: NotificationItem) {
  return render(
    <Provider store={store}>
      <FlowApprovalCard notification={item} />
    </Provider>
  );
}

describe('FlowApprovalCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch({ type: 'notifications/clearAll' });
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
});
