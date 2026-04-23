import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

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
}

const MAX_ITEMS = 200;

const initialState: NotificationState = {
  items: [],
  preferences: { messages: true, agents: true, skills: true, system: true },
};

const notificationSlice = createSlice({
  name: 'notifications',
  initialState,
  reducers: {
    notificationReceived(state, action: PayloadAction<NotificationItem>) {
      const item = action.payload;
      if (!state.preferences[item.category]) return;
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
  },
});

export const selectUnreadCount = (items: NotificationItem[]): number =>
  items.reduce((n, i) => (i.read ? n : n + 1), 0);

export const { notificationReceived, markRead, markAllRead, clearAll, setPreference } =
  notificationSlice.actions;

export default notificationSlice.reducer;
