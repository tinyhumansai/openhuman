import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, {
  type PendingApproval,
  setPendingApprovalForThread,
} from '../../../store/chatRuntimeSlice';
import ApprovalRequestCard from '../ApprovalRequestCard';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const THREAD = 't1';
const approval: PendingApproval = {
  requestId: 'req-approval-1',
  toolName: 'shell',
  message: 'Run `shell` — shell (18 bytes of arguments)',
  command: 'pip show yfinance',
};

function renderCard() {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  store.dispatch(setPendingApprovalForThread({ threadId: THREAD, approval }));
  const utils = render(
    <Provider store={store}>
      <ApprovalRequestCard threadId={THREAD} approval={approval} />
    </Provider>
  );
  return { store, ...utils };
}

describe('ApprovalRequestCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the action summary, exact command, and tool name', () => {
    renderCard();
    expect(screen.getByText('Approval needed')).toBeInTheDocument();
    expect(screen.getByText('Run `shell` — shell (18 bytes of arguments)')).toBeInTheDocument();
    // The exact command being requested is shown verbatim.
    expect(screen.getByText('pip show yfinance')).toBeInTheDocument();
    expect(screen.getByText('shell')).toBeInTheDocument();
  });

  it('uses an opaque warning surface so thread text does not show through', () => {
    renderCard();
    const card = screen.getByRole('alertdialog', { name: 'Approval needed' });
    const command = screen.getByText('pip show yfinance');

    expect(card).toHaveClass('bg-amber-50');
    expect(card).toHaveClass('dark:bg-amber-950');
    expect(card).not.toHaveClass('bg-amber/5');
    expect(card.className).not.toMatch(/\bdark:bg-[^\s/]+\/\d+/);
    expect(command).toHaveClass('dark:bg-surface-canvas');
    expect(command.className).not.toMatch(/\bdark:bg-[^\s/]+\/\d+/);
  });

  it('does not nudge the user to reply yes/no (buttons are the input path)', () => {
    renderCard();
    expect(screen.queryByText(/reply.*yes/i)).not.toBeInTheDocument();
  });

  it('Approve routes approve_once to approval_decide and clears the pending state', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({});
    const { store } = renderCard();

    fireEvent.click(screen.getByText('Approve'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: 'req-approval-1', decision: 'approve_once' },
    });
    await waitFor(() => {
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeUndefined();
    });
  });

  it('Deny routes deny to approval_decide', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({});
    const { store } = renderCard();

    fireEvent.click(screen.getByText('Deny'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: 'req-approval-1', decision: 'deny' },
    });
    await waitFor(() => {
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeUndefined();
    });
  });

  it('Always allow routes approve_always_for_tool to approval_decide and clears the pending state', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({});
    const { store } = renderCard();

    fireEvent.click(screen.getByText('Always allow'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: 'req-approval-1', decision: 'approve_always_for_tool' },
    });
    await waitFor(() => {
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeUndefined();
    });
  });

  it('keeps the prompt and shows an error when the decide RPC fails', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('gate not installed'));
    const { store } = renderCard();

    fireEvent.click(screen.getByText('Approve'));

    await waitFor(() => {
      // Raw RPC error text ('gate not installed') is no longer surfaced to the
      // user — it's kept in a namespaced debug log; the localized fallback shows.
      expect(screen.getByText(/Could not record your decision/)).toBeInTheDocument();
    });
    // Decision failed → approval stays parked, buttons remain actionable.
    expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toEqual(approval);
    expect(screen.getByText('Approve')).toBeInTheDocument();
  });

  it('falls back to the generic prompt when the approval has no message', () => {
    const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
    const noMessage: PendingApproval = { ...approval, message: '' };
    store.dispatch(setPendingApprovalForThread({ threadId: THREAD, approval: noMessage }));
    render(
      <Provider store={store}>
        <ApprovalRequestCard threadId={THREAD} approval={noMessage} />
      </Provider>
    );
    expect(
      screen.getByText('The agent wants to run an action that needs your approval.')
    ).toBeInTheDocument();
  });
});
