import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { addAccount } from '../../store/accountsSlice';
import { __handleFiredForTests, __resetForTests, handleNotificationClick } from './service';

const sampleAccount = {
  id: 'acct1',
  provider: 'slack' as const,
  label: 'Slack',
  createdAt: '2026-01-01T00:00:00Z',
  status: 'open' as const,
};

describe('webviewNotifications service', () => {
  beforeEach(() => {
    __resetForTests();
    store.dispatch(addAccount(sampleAccount));
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('fired events increment unread via Redux', () => {
    const before = store.getState().accounts.unread.acct1 ?? 0;
    __handleFiredForTests({
      account_id: 'acct1',
      provider: 'slack',
      title: 'OpenHuman: Slack — Ping',
      body: 'hi',
      tag: null,
    });
    const after = store.getState().accounts.unread.acct1 ?? 0;
    expect(after).toBe(before + 1);
  });

  it('handleNotificationClick focuses account and clears unread', () => {
    __handleFiredForTests({
      account_id: 'acct1',
      provider: 'slack',
      title: 'OpenHuman: Slack — Ping',
      body: '',
    });
    expect(store.getState().accounts.unread.acct1).toBeGreaterThan(0);

    handleNotificationClick('acct1');
    expect(store.getState().accounts.activeAccountId).toBe('acct1');
    expect(store.getState().accounts.unread.acct1).toBe(0);
  });

  it('fired events for unknown accounts are no-ops', () => {
    __handleFiredForTests({ account_id: 'ghost', provider: 'slack', title: 't', body: 'b' });
    expect(store.getState().accounts.unread.ghost).toBeUndefined();
  });
});
