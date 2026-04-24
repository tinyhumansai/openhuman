import { describe, expect, it } from 'vitest';

import type { IntegrationNotification } from '../../types/notifications';
import notificationsReducer, {
  addNotification,
  markRead,
  setNotifications,
} from '../notificationsSlice';

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

const initialState = { items: [], unreadCount: 0, loading: false, error: null };

describe('notificationsSlice', () => {
  describe('setNotifications', () => {
    it('replaces items and unread count', () => {
      const n1 = makeNotification({ id: 'n-1', status: 'unread' });
      const n2 = makeNotification({ id: 'n-2', status: 'read' });
      const state = notificationsReducer(
        initialState,
        setNotifications({ items: [n1, n2], unread_count: 1 })
      );
      expect(state.items).toHaveLength(2);
      expect(state.unreadCount).toBe(1);
      expect(state.loading).toBe(false);
      expect(state.error).toBeNull();
    });
  });

  describe('markRead', () => {
    it('marks an unread notification as read and decrements unreadCount', () => {
      const n = makeNotification({ id: 'n-1', status: 'unread' });
      const loaded = notificationsReducer(
        initialState,
        setNotifications({ items: [n], unread_count: 1 })
      );
      const state = notificationsReducer(loaded, markRead('n-1'));
      expect(state.items[0].status).toBe('read');
      expect(state.unreadCount).toBe(0);
    });

    it('does not decrement unreadCount below zero', () => {
      const n = makeNotification({ id: 'n-1', status: 'read' });
      const loaded = notificationsReducer(
        initialState,
        setNotifications({ items: [n], unread_count: 0 })
      );
      const state = notificationsReducer(loaded, markRead('n-1'));
      expect(state.unreadCount).toBe(0);
    });

    it('is a no-op for an already-read notification', () => {
      const n = makeNotification({ id: 'n-1', status: 'read' });
      const loaded = notificationsReducer(
        initialState,
        setNotifications({ items: [n], unread_count: 0 })
      );
      const state = notificationsReducer(loaded, markRead('n-1'));
      expect(state.unreadCount).toBe(0);
      expect(state.items[0].status).toBe('read');
    });
  });

  describe('addNotification', () => {
    it('prepends a new unread notification and increments unreadCount', () => {
      const n1 = makeNotification({ id: 'n-1' });
      const loaded = notificationsReducer(
        initialState,
        setNotifications({ items: [n1], unread_count: 1 })
      );
      const n2 = makeNotification({ id: 'n-2', title: 'Second' });
      const state = notificationsReducer(loaded, addNotification(n2));
      expect(state.items[0].id).toBe('n-2');
      expect(state.items).toHaveLength(2);
      expect(state.unreadCount).toBe(2);
    });

    it('does not add a duplicate notification', () => {
      const n = makeNotification({ id: 'n-1' });
      const loaded = notificationsReducer(
        initialState,
        setNotifications({ items: [n], unread_count: 1 })
      );
      const state = notificationsReducer(loaded, addNotification(n));
      expect(state.items).toHaveLength(1);
      expect(state.unreadCount).toBe(1);
    });

    it('does not increment unreadCount for a read notification', () => {
      const n = makeNotification({ id: 'n-2', status: 'read' });
      const state = notificationsReducer(initialState, addNotification(n));
      expect(state.unreadCount).toBe(0);
      expect(state.items).toHaveLength(1);
    });
  });
});
