import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { IntegrationNotification } from '../types/notifications';

export type NotificationCategory = 'messages' | 'agents' | 'skills' | 'system';

export interface NotificationItem {
  id: string;
  category: NotificationCategory;
  title: string;
  body: string;
  timestamp: number;
  read: boolean;
  accountId?: string;
  provider?: string;
  deepLink?: string;
}

export interface NotificationPreferences {
  messages: boolean;
  agents: boolean;
  skills: boolean;
  system: boolean;
}

export interface NotificationState {
  items: NotificationItem[];
  preferences: NotificationPreferences;
  integrationItems: IntegrationNotification[];
  integrationUnreadCount: number;
  integrationLoading: boolean;
  integrationError: string | null;
}

const MAX_ITEMS = 200;

const initialState: NotificationState = {
  items: [],
  preferences: { messages: true, agents: true, skills: true, system: true },
  integrationItems: [],
  integrationUnreadCount: 0,
  integrationLoading: false,
  integrationError: null,
};

const notificationSlice = createSlice({
  name: 'notifications',
  initialState,
  reducers: {
    notificationReceived(state, action: PayloadAction<NotificationItem>) {
      const item = action.payload;
      if (!state.preferences[item.category]) return;
      const existingIndex = state.items.findIndex(i => i.id === item.id);
      if (existingIndex >= 0) {
        // Replace existing entry in place to avoid duplicate rows when
        // socket reconnects or upstream replays the same event id.
        state.items[existingIndex] = item;
        return;
      }
      state.items.unshift(item);
      if (state.items.length > MAX_ITEMS) {
        state.items.length = MAX_ITEMS;
      }
    },
    markRead(state, action: PayloadAction<{ id: string }>) {
      const item = state.items.find(i => i.id === action.payload.id);
      if (item) item.read = true;
    },
    markAllRead(state) {
      for (const item of state.items) item.read = true;
    },
    clearAll(state) {
      state.items = [];
    },
    setPreference(
      state,
      action: PayloadAction<{ category: NotificationCategory; enabled: boolean }>
    ) {
      state.preferences[action.payload.category] = action.payload.enabled;
    },
    setIntegrationLoading(state, action: PayloadAction<boolean>) {
      state.integrationLoading = action.payload;
    },
    setIntegrationError(state, action: PayloadAction<string | null>) {
      state.integrationError = action.payload;
      state.integrationLoading = false;
    },
    setIntegrationNotifications(
      state,
      action: PayloadAction<{ items: IntegrationNotification[]; unread_count: number }>
    ) {
      state.integrationItems = action.payload.items;
      state.integrationUnreadCount = action.payload.unread_count;
      state.integrationLoading = false;
      state.integrationError = null;
    },
    markIntegrationRead(state, action: PayloadAction<string>) {
      const n = state.integrationItems.find(i => i.id === action.payload);
      if (n && n.status === 'unread') {
        n.status = 'read';
        state.integrationUnreadCount = Math.max(0, state.integrationUnreadCount - 1);
      }
    },
    dismissIntegrationNotification(state, action: PayloadAction<string>) {
      const n = state.integrationItems.find(i => i.id === action.payload);
      if (n) {
        const wasUnread = n.status === 'unread';
        n.status = 'dismissed';
        if (wasUnread) {
          state.integrationUnreadCount = Math.max(0, state.integrationUnreadCount - 1);
        }
      }
    },
    addIntegrationNotification(state, action: PayloadAction<IntegrationNotification>) {
      const exists = state.integrationItems.some(i => i.id === action.payload.id);
      if (!exists) {
        state.integrationItems.unshift(action.payload);
        if (action.payload.status === 'unread') {
          state.integrationUnreadCount += 1;
        }
      }
    },
  },
});

export const selectUnreadCount = (items: NotificationItem[]): number =>
  items.reduce((n, i) => (i.read ? n : n + 1), 0);

export const {
  notificationReceived,
  markRead,
  markAllRead,
  clearAll,
  setPreference,
  setIntegrationLoading,
  setIntegrationError,
  setIntegrationNotifications,
  markIntegrationRead,
  dismissIntegrationNotification,
  addIntegrationNotification,
} = notificationSlice.actions;

export { notificationSlice };
export default notificationSlice.reducer;
