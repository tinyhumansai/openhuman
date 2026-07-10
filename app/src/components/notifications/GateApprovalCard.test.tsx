/**
 * GateApprovalCard (flow-approval surface — notifications) — rendering +
 * decision contract for the `flow-gate-approval` CoreNotification kind.
 * Asserts: renders as an alertdialog with tool + summary; Approve once /
 * Approve always / Deny each route the matching decision to
 * `openhuman.approval_decide` reading `{ request_id }` from the action
 * payload; a successful decision marks the notification read and clears its
 * actions; a failed decision surfaces a localized error and re-enables the
 * buttons without touching the notification; and an invalid payload is
 * handled defensively (error shown, no RPC call). Mirrors
 * `FlowApprovalCard.test.tsx`'s approach (real Redux `store`, dispatch
 * `notificationReceived` directly).
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { type NotificationItem } from '../../store/notificationSlice';
import GateApprovalCard, { isGateApprovalNotification } from './GateApprovalCard';

const decideApproval = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/approvalApi', () => ({ decideApproval }));

function makeItem(overrides: Partial<NotificationItem> = {}): NotificationItem {
  return {
    id: 'flow-gate-approval:req-1',
    kind: 'flow-gate-approval',
    category: 'agents',
    title: 'Workflow needs approval',
    body: '',
    timestamp: Date.now(),
    read: false,
    actions: [
      {
        actionId: 'decide',
        label: 'Review',
        payload: {
          request_id: 'req-1',
          flow_id: 'flow-1',
          tool_name: 'shell',
          summary: 'Run `shell` — rm -rf /tmp/scratch',
        },
      },
    ],
    ...overrides,
  };
}

function renderCard(item: NotificationItem) {
  return render(
    <Provider store={store}>
      <GateApprovalCard notification={item} />
    </Provider>
  );
}

describe('isGateApprovalNotification', () => {
  it('matches on the kind field', () => {
    expect(isGateApprovalNotification({ id: 'anything', kind: 'flow-gate-approval' })).toBe(true);
  });

  it('matches on the legacy id-prefix fallback', () => {
    expect(isGateApprovalNotification({ id: 'flow-gate-approval:req-1', kind: undefined })).toBe(
      true
    );
  });

  it('does not match unrelated notifications', () => {
    expect(
      isGateApprovalNotification({ id: 'flow-pending-approval:flow-1:t1', kind: undefined })
    ).toBe(false);
  });
});

describe('GateApprovalCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch({ type: 'notifications/clearAll' });
  });

  it('renders as an alertdialog with the summary and tool name', () => {
    renderCard(makeItem());
    const card = screen.getByTestId('gate-approval-card');
    expect(card).toHaveAttribute('role', 'alertdialog');
    expect(screen.getByText('Run `shell` — rm -rf /tmp/scratch')).toBeInTheDocument();
    expect(screen.getByText('shell')).toBeInTheDocument();
  });

  it('renders all three decision buttons', () => {
    renderCard(makeItem());
    expect(screen.getByTestId('gate-approval-approve')).toBeInTheDocument();
    expect(screen.getByTestId('gate-approval-always')).toBeInTheDocument();
    expect(screen.getByTestId('gate-approval-deny')).toBeInTheDocument();
  });

  it('Approve once calls approval_decide with the request id and marks the notification read', async () => {
    decideApproval.mockResolvedValue(undefined);
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('gate-approval-approve'));

    await waitFor(() => expect(decideApproval).toHaveBeenCalledWith('req-1', 'approve_once'));
    await waitFor(() => {
      const item = store
        .getState()
        .notifications.items.find(i => i.id === 'flow-gate-approval:req-1');
      expect(item?.read).toBe(true);
      expect(item?.actions ?? []).toHaveLength(0);
    });
  });

  it('Approve always calls approval_decide with approve_always_for_flow', async () => {
    decideApproval.mockResolvedValue(undefined);
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('gate-approval-always'));

    await waitFor(() =>
      expect(decideApproval).toHaveBeenCalledWith('req-1', 'approve_always_for_flow')
    );
  });

  it('Deny calls approval_decide with deny', async () => {
    decideApproval.mockResolvedValue(undefined);
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('gate-approval-deny'));

    await waitFor(() => expect(decideApproval).toHaveBeenCalledWith('req-1', 'deny'));
  });

  it('shows a localized error and re-enables the buttons when the decide RPC fails', async () => {
    decideApproval.mockRejectedValue(new Error('gate not installed'));
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('gate-approval-approve'));

    await waitFor(() => {
      expect(screen.getByTestId('gate-approval-approve')).not.toBeDisabled();
    });
    expect(
      screen.getByText(
        (_content, element) =>
          element?.tagName.toLowerCase() === 'p' &&
          (element?.textContent ?? '').includes('Could not record your decision')
      )
    ).toBeInTheDocument();
    const item = store
      .getState()
      .notifications.items.find(i => i.id === 'flow-gate-approval:req-1');
    expect(item?.actions).toHaveLength(1);
  });

  it('treats a missing payload as invalid (shows error, no RPC call)', async () => {
    renderCard(
      makeItem({ actions: [{ actionId: 'decide', label: 'Review', payload: undefined }] })
    );

    fireEvent.click(screen.getByTestId('gate-approval-approve'));

    await waitFor(() => {
      expect(
        screen.getByText(
          (_content, element) =>
            element?.tagName.toLowerCase() === 'p' &&
            (element?.textContent ?? '').includes('Could not record your decision')
        )
      ).toBeInTheDocument();
    });
    expect(decideApproval).not.toHaveBeenCalled();
  });

  it('disables all buttons while a decision is in flight', async () => {
    let resolve!: (v: unknown) => void;
    decideApproval.mockImplementation(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    renderCard(makeItem());

    fireEvent.click(screen.getByTestId('gate-approval-approve'));

    expect(screen.getByTestId('gate-approval-approve')).toBeDisabled();
    expect(screen.getByTestId('gate-approval-always')).toBeDisabled();
    expect(screen.getByTestId('gate-approval-deny')).toBeDisabled();

    resolve(undefined);
    await waitFor(() => expect(screen.getByTestId('gate-approval-approve')).not.toBeDisabled());
  });
});
