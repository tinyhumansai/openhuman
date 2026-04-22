import { describe, expect, it } from 'vitest';

import reducer, {
  addAccount,
  focusAccountFromNotification,
  noteWebviewNotificationFired,
} from '../accountsSlice';

const sampleAccount = {
  id: 'acct1',
  provider: 'slack' as const,
  label: 'Slack',
  createdAt: '2026-01-01T00:00:00Z',
  status: 'open' as const,
};

describe('accountsSlice webview-notification actions', () => {
  it('noteWebviewNotificationFired increments unread for known accounts', () => {
    const added = reducer(undefined, addAccount(sampleAccount));
    const fired = reducer(added, noteWebviewNotificationFired({ accountId: 'acct1' }));
    const firedTwice = reducer(fired, noteWebviewNotificationFired({ accountId: 'acct1' }));
    expect(firedTwice.unread.acct1).toBe(2);
  });

  it('noteWebviewNotificationFired ignores unknown accounts', () => {
    const fired = reducer(undefined, noteWebviewNotificationFired({ accountId: 'ghost' }));
    expect(fired.unread.ghost).toBeUndefined();
  });

  it('focusAccountFromNotification sets active account and clears unread', () => {
    let state = reducer(undefined, addAccount(sampleAccount));
    state = reducer(state, noteWebviewNotificationFired({ accountId: 'acct1' }));
    state = reducer(state, noteWebviewNotificationFired({ accountId: 'acct1' }));
    expect(state.unread.acct1).toBe(2);

    state = reducer(state, focusAccountFromNotification({ accountId: 'acct1' }));
    expect(state.activeAccountId).toBe('acct1');
    expect(state.unread.acct1).toBe(0);
  });

  it('focusAccountFromNotification ignores unknown accounts', () => {
    const state = reducer(undefined, focusAccountFromNotification({ accountId: 'ghost' }));
    expect(state.activeAccountId).toBeNull();
  });
});
