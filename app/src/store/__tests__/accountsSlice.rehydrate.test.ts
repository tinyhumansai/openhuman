/**
 * Issue #1379 — embedded-app tabs (Slack/Discord/WhatsApp/...) showing
 * "is taking longer than expected" overlay immediately after reopening
 * the desktop app was caused by Redux-persist replaying the previous
 * session's transient `Account.status` (`timeout` / `loading` / ...)
 * before the new webview spawn had even started. The fix is a
 * `REHYDRATE` extraReducer that flips any non-`closed` status to
 * `closed` so the next session begins from a fresh load state.
 */
import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import type { Account, AccountsState, AccountStatus } from '../../types/accounts';
import reducer from '../accountsSlice';

function makeAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: 'acct-1',
    provider: 'slack',
    label: 'Slack',
    createdAt: '2026-01-01T00:00:00Z',
    status: 'pending',
    ...overrides,
  };
}

function seedState(accounts: Account[]): AccountsState {
  const state: AccountsState = {
    accounts: {},
    order: [],
    activeAccountId: accounts[0]?.id ?? null,
    lastActiveAccountId: accounts[0]?.id ?? null,
    messages: {},
    unread: {},
    logs: {},
  };
  for (const acct of accounts) {
    state.accounts[acct.id] = acct;
    state.order.push(acct.id);
    state.messages[acct.id] = [];
    state.unread[acct.id] = 0;
    state.logs[acct.id] = [];
  }
  return state;
}

function rehydrate(state: AccountsState, key = 'accounts') {
  return reducer(state, { type: REHYDRATE, key, payload: state } as unknown as {
    type: typeof REHYDRATE;
  });
}

describe('accountsSlice REHYDRATE — issue #1379', () => {
  const TRANSIENT: AccountStatus[] = ['pending', 'loading', 'timeout', 'open', 'error'];

  it.each(TRANSIENT)('resets `%s` status to `closed` so stale overlays do not replay', status => {
    const before = seedState([makeAccount({ status, lastError: 'stale' })]);
    const after = rehydrate(before);
    expect(after.accounts['acct-1']?.status).toBe('closed');
    expect(after.accounts['acct-1']?.lastError).toBeUndefined();
  });

  it('leaves accounts already in `closed` untouched', () => {
    const before = seedState([makeAccount({ status: 'closed' })]);
    const after = rehydrate(before);
    expect(after.accounts['acct-1']?.status).toBe('closed');
  });

  it('resets every account in the directory, not just the active one', () => {
    const before = seedState([
      makeAccount({ id: 'acct-slack', provider: 'slack', status: 'timeout' }),
      makeAccount({ id: 'acct-discord', provider: 'discord', status: 'loading' }),
      makeAccount({ id: 'acct-tg', provider: 'telegram', status: 'closed' }),
    ]);
    const after = rehydrate(before);
    expect(after.accounts['acct-slack']?.status).toBe('closed');
    expect(after.accounts['acct-discord']?.status).toBe('closed');
    expect(after.accounts['acct-tg']?.status).toBe('closed');
  });

  it('preserves the persisted account directory, order, and MRU pointer', () => {
    const before = seedState([
      makeAccount({ id: 'acct-slack', provider: 'slack', status: 'timeout', label: 'Work Slack' }),
      makeAccount({ id: 'acct-discord', provider: 'discord', status: 'open' }),
    ]);
    before.activeAccountId = 'acct-slack';
    before.lastActiveAccountId = 'acct-discord';

    const after = rehydrate(before);
    expect(after.order).toEqual(['acct-slack', 'acct-discord']);
    // Issue #2044 — activeAccountId is cleared on rehydrate (see below).
    expect(after.activeAccountId).toBeNull();
    expect(after.lastActiveAccountId).toBe('acct-discord');
    expect(after.accounts['acct-slack']?.label).toBe('Work Slack');
    expect(after.accounts['acct-slack']?.provider).toBe('slack');
    expect(after.accounts['acct-discord']?.provider).toBe('discord');
  });

  it('ignores REHYDRATE actions for other persist keys', () => {
    const before = seedState([makeAccount({ status: 'timeout' })]);
    const after = rehydrate(before, 'notifications');
    expect(after.accounts['acct-1']?.status).toBe('timeout');
  });

  it('is a no-op when no accounts are persisted', () => {
    const before = seedState([]);
    const after = rehydrate(before);
    expect(after.accounts).toEqual({});
    expect(after.order).toEqual([]);
  });
});

describe('accountsSlice REHYDRATE — issue #2044 (activeAccountId not persisted)', () => {
  it('clears a non-null activeAccountId on rehydrate so no webview auto-surfaces', () => {
    const before = seedState([
      makeAccount({ id: 'acct-slack', provider: 'slack', status: 'closed' }),
    ]);
    before.activeAccountId = 'acct-slack';
    before.lastActiveAccountId = 'acct-slack';

    const after = rehydrate(before);
    expect(after.activeAccountId).toBeNull();
    // MRU pointer is intentionally preserved so the off-screen prewarm
    // can still warm the same account in the background.
    expect(after.lastActiveAccountId).toBe('acct-slack');
  });

  it('leaves activeAccountId null when nothing was persisted', () => {
    const before = seedState([]);
    before.activeAccountId = null;
    const after = rehydrate(before);
    expect(after.activeAccountId).toBeNull();
  });

  it('does not touch activeAccountId for REHYDRATE actions on other persist keys', () => {
    const before = seedState([
      makeAccount({ id: 'acct-slack', provider: 'slack', status: 'closed' }),
    ]);
    before.activeAccountId = 'acct-slack';
    const after = rehydrate(before, 'notifications');
    expect(after.activeAccountId).toBe('acct-slack');
  });
});
