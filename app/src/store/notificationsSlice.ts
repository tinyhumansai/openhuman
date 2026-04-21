import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { IntegrationNotification } from '../types/notifications';

interface NotificationsState {
  items: IntegrationNotification[];
  unreadCount: number;
  loading: boolean;
  error: string | null;
}

const initialState: NotificationsState = { items: [], unreadCount: 0, loading: false, error: null };

export const notificationsSlice = createSlice({
  name: 'notifications',
  initialState,
  reducers: {
    setNotificationsLoading(state, _action: PayloadAction<boolean>) {
      state.loading = _action.payload;
    },
    setNotificationsError(state, action: PayloadAction<string | null>) {
      state.error = action.payload;
      state.loading = false;
    },
    setNotifications(
      state,
      action: PayloadAction<{ items: IntegrationNotification[]; unread_count: number }>
    ) {
      state.items = action.payload.items;
      state.unreadCount = action.payload.unread_count;
      state.loading = false;
      state.error = null;
    },
    markRead(state, action: PayloadAction<string>) {
      const n = state.items.find(i => i.id === action.payload);
      if (n && n.status === 'unread') {
        n.status = 'read';
        state.unreadCount = Math.max(0, state.unreadCount - 1);
      }
    },
    addNotification(state, action: PayloadAction<IntegrationNotification>) {
      // Prepend so newest appears first; avoid duplicates by id.
      const exists = state.items.some(i => i.id === action.payload.id);
      if (!exists) {
        state.items.unshift(action.payload);
        if (action.payload.status === 'unread') {
          state.unreadCount += 1;
        }
      }
    },
  },
});

export const {
  setNotificationsLoading,
  setNotificationsError,
  setNotifications,
  markRead,
  addNotification,
} = notificationsSlice.actions;

export default notificationsSlice.reducer;
