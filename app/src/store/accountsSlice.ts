import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type {
  Account,
  AccountLogEntry,
  AccountsState,
  AccountStatus,
  IngestedMessage,
} from '../types/accounts';

const MAX_MESSAGES_PER_ACCOUNT = 200;
const MAX_LOG_LINES_PER_ACCOUNT = 100;

const initialState: AccountsState = {
  accounts: {},
  order: [],
  activeAccountId: null,
  messages: {},
  unread: {},
  logs: {},
};

const accountsSlice = createSlice({
  name: 'accounts',
  initialState,
  reducers: {
    addAccount(state, action: PayloadAction<Account>) {
      const acct = action.payload;
      if (!state.accounts[acct.id]) {
        state.order.push(acct.id);
      }
      state.accounts[acct.id] = acct;
      state.messages[acct.id] ??= [];
      state.unread[acct.id] ??= 0;
      state.logs[acct.id] ??= [];
      state.activeAccountId ??= acct.id;
    },

    removeAccount(state, action: PayloadAction<{ accountId: string }>) {
      const { accountId } = action.payload;
      delete state.accounts[accountId];
      delete state.messages[accountId];
      delete state.unread[accountId];
      delete state.logs[accountId];
      state.order = state.order.filter(id => id !== accountId);
      if (state.activeAccountId === accountId) {
        state.activeAccountId = state.order[0] ?? null;
      }
    },

    setActiveAccount(state, action: PayloadAction<string | null>) {
      state.activeAccountId = action.payload;
    },

    setAccountStatus(
      state,
      action: PayloadAction<{ accountId: string; status: AccountStatus; lastError?: string }>
    ) {
      const acct = state.accounts[action.payload.accountId];
      if (!acct) return;
      acct.status = action.payload.status;
      acct.lastError = action.payload.lastError;
    },

    appendMessages(
      state,
      action: PayloadAction<{ accountId: string; messages: IngestedMessage[]; unread?: number }>
    ) {
      const { accountId, messages, unread } = action.payload;
      if (!state.accounts[accountId]) return;
      const list = (state.messages[accountId] ??= []);
      // Replace the snapshot entirely — recipes ingest the visible chat list,
      // not deltas, so the latest scrape is the truth. Cap to avoid runaway.
      const next = messages.slice(0, MAX_MESSAGES_PER_ACCOUNT);
      list.length = 0;
      list.push(...next);
      if (typeof unread === 'number') {
        state.unread[accountId] = unread;
      }
    },

    appendLog(state, action: PayloadAction<{ accountId: string; entry: AccountLogEntry }>) {
      const { accountId, entry } = action.payload;
      const list = (state.logs[accountId] ??= []);
      list.push(entry);
      if (list.length > MAX_LOG_LINES_PER_ACCOUNT) {
        list.splice(0, list.length - MAX_LOG_LINES_PER_ACCOUNT);
      }
    },

    noteWebviewNotificationFired(state, action: PayloadAction<{ accountId: string }>) {
      const { accountId } = action.payload;
      if (!state.accounts[accountId]) return;
      state.unread[accountId] = (state.unread[accountId] ?? 0) + 1;
    },

    focusAccountFromNotification(state, action: PayloadAction<{ accountId: string }>) {
      const { accountId } = action.payload;
      if (!state.accounts[accountId]) return;
      state.activeAccountId = accountId;
      state.unread[accountId] = 0;
    },

    resetAccountsState() {
      return initialState;
    },
  },
});

export const {
  addAccount,
  removeAccount,
  setActiveAccount,
  setAccountStatus,
  appendMessages,
  appendLog,
  noteWebviewNotificationFired,
  focusAccountFromNotification,
  resetAccountsState,
} = accountsSlice.actions;

export default accountsSlice.reducer;
