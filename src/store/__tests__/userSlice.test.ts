import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import type { User } from '../../types/api';
import userReducer, { clearUser, setUser } from '../userSlice';

function createStore() {
  return configureStore({ reducer: { user: userReducer } });
}

const mockUser: User = {
  _id: 'user-123',
  telegramId: 12345678,
  hasAccess: true,
  magicWord: 'alpha',
  firstName: 'Test',
  lastName: 'User',
  username: 'testuser',
  role: 'user',
  activeTeamId: 'team-1',
  referral: {},
  subscription: { hasActiveSubscription: false, plan: 'FREE' },
  settings: {
    dailySummariesEnabled: false,
    dailySummaryChatIds: [],
    autoCompleteEnabled: false,
    autoCompleteVisibility: 'always',
    autoCompleteWhitelistChatIds: [],
    autoCompleteBlacklistChatIds: [],
  },
  usage: { cycleBudgetUsd: 10, spentThisCycleUsd: 0, spentTodayUsd: 0, cycleStartDate: new Date() },
  autoDeleteTelegramMessagesAfterDays: 30,
  autoDeleteThreadsAfterDays: 30,
};

describe('userSlice', () => {
  it('starts with null user', () => {
    const store = createStore();
    expect(store.getState().user.user).toBeNull();
    expect(store.getState().user.isLoading).toBe(false);
    expect(store.getState().user.error).toBeNull();
  });

  it('sets user via setUser', () => {
    const store = createStore();
    store.dispatch(setUser(mockUser));
    expect(store.getState().user.user).toEqual(mockUser);
    expect(store.getState().user.error).toBeNull();
  });

  it('clears user via clearUser', () => {
    const store = createStore();
    store.dispatch(setUser(mockUser));
    store.dispatch(clearUser());
    expect(store.getState().user.user).toBeNull();
    expect(store.getState().user.isLoading).toBe(false);
    expect(store.getState().user.error).toBeNull();
  });

  it('sets user to null via setUser(null)', () => {
    const store = createStore();
    store.dispatch(setUser(mockUser));
    store.dispatch(setUser(null));
    expect(store.getState().user.user).toBeNull();
  });
});
