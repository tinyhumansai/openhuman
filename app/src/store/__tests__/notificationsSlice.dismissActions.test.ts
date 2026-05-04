import { describe, expect, it } from 'vitest';

import type { IntegrationNotification } from '../../types/notifications';
import notificationReducer, {
  dismissIntegrationNotification,
  setIntegrationNotifications,
} from '../notificationSlice';

const makeNotification = (
  overrides: Partial<IntegrationNotification> = {}
): IntegrationNotification => ({
  id: 'n-1',
  provider: 'slack',
  title: 'Test notification',
  body: 'Hello',
  raw_payload: {},
  status: 'unread',
  received_at: '2026-04-23T00:00:00Z',
  ...overrides,
});

const baseState = notificationReducer(undefined, { type: '@@init' });
const initialState = {
  ...baseState,
  integrationItems: [],
  integrationUnreadCount: 0,
  integrationLoading: false,
  integrationError: null,
};

describe('notificationSlice — dismissIntegrationNotification', () => {
  it('marks unread notifications as dismissed and decrements unread count', () => {
    const unread = makeNotification({ id: 'n-unread', status: 'unread' });
    const loaded = notificationReducer(
      initialState,
      setIntegrationNotifications({ items: [unread], unread_count: 1 })
    );

    const state = notificationReducer(loaded, dismissIntegrationNotification('n-unread'));

    expect(state.integrationItems[0].status).toBe('dismissed');
    expect(state.integrationUnreadCount).toBe(0);
  });

  it('marks read notifications as dismissed without changing unread count', () => {
    const read = makeNotification({ id: 'n-read', status: 'read' });
    const loaded = notificationReducer(
      initialState,
      setIntegrationNotifications({ items: [read], unread_count: 0 })
    );

    const state = notificationReducer(loaded, dismissIntegrationNotification('n-read'));

    expect(state.integrationItems[0].status).toBe('dismissed');
    expect(state.integrationUnreadCount).toBe(0);
  });

  it('is a no-op when the id does not exist', () => {
    const item = makeNotification({ id: 'n-1', status: 'unread' });
    const loaded = notificationReducer(
      initialState,
      setIntegrationNotifications({ items: [item], unread_count: 1 })
    );

    const state = notificationReducer(loaded, dismissIntegrationNotification('does-not-exist'));

    expect(state.integrationItems[0].status).toBe('unread');
    expect(state.integrationUnreadCount).toBe(1);
  });

  it('does not decrement unread count when dismissing an already dismissed item', () => {
    const dismissed = makeNotification({ id: 'n-dismissed', status: 'dismissed' });
    const loaded = notificationReducer(
      initialState,
      setIntegrationNotifications({ items: [dismissed], unread_count: 0 })
    );

    const state = notificationReducer(loaded, dismissIntegrationNotification('n-dismissed'));

    expect(state.integrationItems[0].status).toBe('dismissed');
    expect(state.integrationUnreadCount).toBe(0);
  });
});
