/**
 * Action-button contract for core notification cards (issue #3507).
 * Asserts that the calendar auto-join prompt renders its buttons, dispatches
 * the `openhuman.agent_meetings_notification_action` RPC with the right
 * params on click, and surfaces an error when the RPC rejects.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { type NotificationItem } from '../../store/notificationSlice';
import CoreNotificationCard from './CoreNotificationCard';

const callCoreRpc = vi.hoisted(() => vi.fn());
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc }));

function makeItem(overrides: Partial<NotificationItem> = {}): NotificationItem {
  return {
    id: 'meet-auto-join:m1',
    category: 'meetings',
    title: 'Meeting starting: Standup',
    body: 'Add Tiny to this meeting?',
    timestamp: Date.now(),
    read: false,
    actions: [
      { actionId: 'join_listen', label: 'Join (listen only)', payload: { meetingId: 'm1' } },
      { actionId: 'join_active', label: 'Join & reply', payload: { meetingId: 'm1' } },
      { actionId: 'skip', label: 'Not this one', payload: { meetingId: 'm1' } },
      { actionId: 'always_join', label: 'Always join', payload: { meetingId: 'm1' } },
    ],
    ...overrides,
  };
}

function renderCard(item: NotificationItem) {
  return render(
    <Provider store={store}>
      <CoreNotificationCard notification={item} />
    </Provider>
  );
}

describe('CoreNotificationCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch({ type: 'notifications/clearAll' });
  });

  it('renders a localized button per action', () => {
    renderCard(makeItem());
    expect(screen.getByRole('button', { name: 'Join (listen only)' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Join & reply' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Not this one' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Always join' })).toBeInTheDocument();
  });

  it('dispatches the notification-action RPC with action_id + payload on click', async () => {
    callCoreRpc.mockResolvedValue({ ok: true });
    renderCard(makeItem());

    fireEvent.click(screen.getByRole('button', { name: 'Join (listen only)' }));

    await waitFor(() => expect(callCoreRpc).toHaveBeenCalledTimes(1));
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.agent_meetings_notification_action',
      params: { action_id: 'join_listen', payload: { meetingId: 'm1' } },
    });
  });

  it('marks the notification read in the store after a successful action', async () => {
    callCoreRpc.mockResolvedValue({ ok: true });
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByRole('button', { name: 'Not this one' }));

    await waitFor(() => {
      const item = store.getState().notifications.items.find(i => i.id === 'meet-auto-join:m1');
      expect(item?.read).toBe(true);
    });
  });

  it('clears the action buttons after a successful action so it cannot be re-clicked', async () => {
    callCoreRpc.mockResolvedValue({ ok: true });
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByRole('button', { name: 'Not this one' }));

    await waitFor(() => {
      const item = store.getState().notifications.items.find(i => i.id === 'meet-auto-join:m1');
      expect(item?.actions ?? []).toHaveLength(0);
    });
  });

  it('keeps the action buttons when the RPC rejects', async () => {
    callCoreRpc.mockRejectedValue(new Error('boom'));
    store.dispatch({ type: 'notifications/notificationReceived', payload: makeItem() });
    renderCard(makeItem());

    fireEvent.click(screen.getByRole('button', { name: 'Not this one' }));

    await waitFor(() =>
      expect(
        screen.getByText('Could not complete that action. Please try again.')
      ).toBeInTheDocument()
    );
    const item = store.getState().notifications.items.find(i => i.id === 'meet-auto-join:m1');
    expect(item?.actions).toHaveLength(4);
  });

  it('surfaces an error message when the RPC rejects', async () => {
    callCoreRpc.mockRejectedValue(new Error('boom'));
    renderCard(makeItem());

    fireEvent.click(screen.getByRole('button', { name: 'Always join' }));

    await waitFor(() =>
      expect(
        screen.getByText('Could not complete that action. Please try again.')
      ).toBeInTheDocument()
    );
  });

  it('renders nothing actionable when there are no actions', () => {
    renderCard(makeItem({ actions: [] }));
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('disables all buttons while an action RPC is in flight', async () => {
    let resolve!: (v: unknown) => void;
    callCoreRpc.mockImplementation(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    renderCard(makeItem());

    fireEvent.click(screen.getByRole('button', { name: 'Join (listen only)' }));

    // All buttons should be disabled while the call is pending.
    const buttons = screen.getAllByRole('button');
    buttons.forEach(btn => expect(btn).toBeDisabled());

    resolve({ ok: true });
    await waitFor(() => buttons.forEach(btn => expect(btn).not.toBeDisabled()));
  });

  it('shows no unread dot when notification is already read', () => {
    renderCard(makeItem({ read: true }));
    expect(document.querySelector('[aria-hidden="true"]')).toBeNull();
  });

  it('shows unread dot when notification is unread', () => {
    renderCard(makeItem({ read: false }));
    expect(document.querySelector('[aria-hidden="true"]')).toBeInTheDocument();
  });
});
