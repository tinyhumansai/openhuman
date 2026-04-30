import { describe, expect, it } from 'vitest';

import type { IntegrationNotification } from '../../types/notifications';
import notificationReducer, {
  addIntegrationNotification,
  dismissIntegrationNotification,
  markIntegrationActed,
  markIntegrationRead,
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

describe('notificationSlice — integration notifications', () => {
  describe('setIntegrationNotifications', () => {
    it('replaces items and unread count', () => {
      const n1 = makeNotification({ id: 'n-1', status: 'unread' });
      const n2 = makeNotification({ id: 'n-2', status: 'read' });
      const state = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n1, n2], unread_count: 1 })
      );
      expect(state.integrationItems).toHaveLength(2);
      expect(state.integrationUnreadCount).toBe(1);
      expect(state.integrationLoading).toBe(false);
      expect(state.integrationError).toBeNull();
    });
  });

  describe('markIntegrationRead', () => {
    it('marks an unread notification as read and decrements integrationUnreadCount', () => {
      const n = makeNotification({ id: 'n-1', status: 'unread' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 1 })
      );
      const state = notificationReducer(loaded, markIntegrationRead('n-1'));
      expect(state.integrationItems[0].status).toBe('read');
      expect(state.integrationUnreadCount).toBe(0);
    });

    it('does not decrement integrationUnreadCount below zero', () => {
      const n = makeNotification({ id: 'n-1', status: 'read' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 0 })
      );
      const state = notificationReducer(loaded, markIntegrationRead('n-1'));
      expect(state.integrationUnreadCount).toBe(0);
    });

    it('is a no-op for an already-read notification', () => {
      const n = makeNotification({ id: 'n-1', status: 'read' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 0 })
      );
      const state = notificationReducer(loaded, markIntegrationRead('n-1'));
      expect(state.integrationUnreadCount).toBe(0);
      expect(state.integrationItems[0].status).toBe('read');
    });
  });

  describe('markIntegrationActed', () => {
    it('sets status to acted and decrements unread count for an unread notification', () => {
      const n = makeNotification({ id: 'n-1', status: 'unread' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 1 })
      );
      const state = notificationReducer(loaded, markIntegrationActed('n-1'));
      expect(state.integrationItems[0].status).toBe('acted');
      expect(state.integrationUnreadCount).toBe(0);
    });

    it('sets status to acted without changing unread count for a read notification', () => {
      const n = makeNotification({ id: 'n-1', status: 'read' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 0 })
      );
      const state = notificationReducer(loaded, markIntegrationActed('n-1'));
      expect(state.integrationItems[0].status).toBe('acted');
      expect(state.integrationUnreadCount).toBe(0);
    });

    it('is a no-op for unknown id', () => {
      const n = makeNotification({ id: 'n-1', status: 'unread' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 1 })
      );
      const state = notificationReducer(loaded, markIntegrationActed('does-not-exist'));
      expect(state.integrationUnreadCount).toBe(1);
      expect(state.integrationItems[0].status).toBe('unread');
    });
  });

  describe('addIntegrationNotification', () => {
    it('prepends a new unread notification and increments integrationUnreadCount', () => {
      const n1 = makeNotification({ id: 'n-1' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n1], unread_count: 1 })
      );
      const n2 = makeNotification({ id: 'n-2', title: 'Second' });
      const state = notificationReducer(loaded, addIntegrationNotification(n2));
      expect(state.integrationItems[0].id).toBe('n-2');
      expect(state.integrationItems).toHaveLength(2);
      expect(state.integrationUnreadCount).toBe(2);
    });

    it('does not add a duplicate notification', () => {
      const n = makeNotification({ id: 'n-1' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 1 })
      );
      const state = notificationReducer(loaded, addIntegrationNotification(n));
      expect(state.integrationItems).toHaveLength(1);
      expect(state.integrationUnreadCount).toBe(1);
    });

    it('does not increment integrationUnreadCount for a read notification', () => {
      const n = makeNotification({ id: 'n-2', status: 'read' });
      const state = notificationReducer(initialState, addIntegrationNotification(n));
      expect(state.integrationUnreadCount).toBe(0);
      expect(state.integrationItems).toHaveLength(1);
    });
  });

  describe('dismissIntegrationNotification', () => {
    it('marks unread notification as dismissed and decrements unread count', () => {
      const n = makeNotification({ id: 'n-1', status: 'unread' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 1 })
      );

      const state = notificationReducer(loaded, dismissIntegrationNotification('n-1'));
      expect(state.integrationItems[0].status).toBe('dismissed');
      expect(state.integrationUnreadCount).toBe(0);
    });

    it('does not decrement unread count below zero', () => {
      const n = makeNotification({ id: 'n-1', status: 'dismissed' });
      const loaded = notificationReducer(
        initialState,
        setIntegrationNotifications({ items: [n], unread_count: 0 })
      );

      const state = notificationReducer(loaded, dismissIntegrationNotification('n-1'));
      expect(state.integrationItems[0].status).toBe('dismissed');
      expect(state.integrationUnreadCount).toBe(0);
    });
  });
});
