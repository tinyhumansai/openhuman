import { describe, expect, it } from 'vitest';

import type { Account, AccountLogEntry, IngestedMessage } from '../../types/accounts';
import reducer, {
  addAccount,
  appendLog,
  appendMessages,
  removeAccount,
  resetAccountsState,
  setAccountStatus,
  setActiveAccount,
} from '../accountsSlice';

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

function makeMessage(overrides: Partial<IngestedMessage> = {}): IngestedMessage {
  return { id: 'm-1', from: 'alice', body: 'hi', unread: 0, ts: 0, ...overrides };
}

describe('accountsSlice addAccount', () => {
  it('inserts a new account, initialises caches, and picks the first as active', () => {
    const state = reducer(undefined, addAccount(makeAccount()));
    expect(state.accounts['acct-1']).toBeDefined();
    expect(state.order).toEqual(['acct-1']);
    expect(state.messages['acct-1']).toEqual([]);
    expect(state.unread['acct-1']).toBe(0);
    expect(state.logs['acct-1']).toEqual([]);
    expect(state.activeAccountId).toBe('acct-1');
  });

  it('re-adding an existing account updates fields but does not duplicate order or reset caches', () => {
    let state = reducer(undefined, addAccount(makeAccount()));
    state = reducer(
      state,
      appendMessages({ accountId: 'acct-1', messages: [makeMessage()], unread: 3 })
    );
    expect(state.messages['acct-1']).toHaveLength(1);
    expect(state.unread['acct-1']).toBe(3);

    state = reducer(state, addAccount(makeAccount({ label: 'Slack Renamed', status: 'open' })));
    expect(state.order).toEqual(['acct-1']);
    expect(state.accounts['acct-1'].label).toBe('Slack Renamed');
    expect(state.accounts['acct-1'].status).toBe('open');
    // nullish-coalescing assignments must preserve existing caches.
    expect(state.messages['acct-1']).toHaveLength(1);
    expect(state.unread['acct-1']).toBe(3);
  });

  it('does not overwrite activeAccountId when more accounts are added', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(state, addAccount(makeAccount({ id: 'b' })));
    expect(state.activeAccountId).toBe('a');
    expect(state.order).toEqual(['a', 'b']);
  });
});

describe('accountsSlice removeAccount', () => {
  it('removes account and associated caches', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(state, addAccount(makeAccount({ id: 'b' })));
    state = reducer(
      state,
      appendLog({ accountId: 'a', entry: { ts: 0, level: 'info', msg: 'x' } as AccountLogEntry })
    );

    state = reducer(state, removeAccount({ accountId: 'a' }));
    expect(state.accounts.a).toBeUndefined();
    expect(state.messages.a).toBeUndefined();
    expect(state.unread.a).toBeUndefined();
    expect(state.logs.a).toBeUndefined();
    expect(state.order).toEqual(['b']);
  });

  it('reassigns active to the first remaining account on removal of active', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(state, addAccount(makeAccount({ id: 'b' })));
    expect(state.activeAccountId).toBe('a');

    state = reducer(state, removeAccount({ accountId: 'a' }));
    expect(state.activeAccountId).toBe('b');
  });

  it('sets active to null when last account is removed', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'only' })));
    state = reducer(state, removeAccount({ accountId: 'only' }));
    expect(state.order).toEqual([]);
    expect(state.activeAccountId).toBeNull();
  });
});

describe('accountsSlice setActiveAccount and setAccountStatus', () => {
  it('setActiveAccount sets and clears the active id', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(state, setActiveAccount(null));
    expect(state.activeAccountId).toBeNull();
    state = reducer(state, setActiveAccount('a'));
    expect(state.activeAccountId).toBe('a');
  });

  it('setAccountStatus updates status and lastError', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(
      state,
      setAccountStatus({ accountId: 'a', status: 'error', lastError: 'sso expired' })
    );
    expect(state.accounts.a.status).toBe('error');
    expect(state.accounts.a.lastError).toBe('sso expired');
  });

  it('setAccountStatus ignores unknown accounts', () => {
    const state = reducer(undefined, setAccountStatus({ accountId: 'ghost', status: 'error' }));
    expect(state.accounts.ghost).toBeUndefined();
  });
});

describe('accountsSlice appendMessages', () => {
  it('replaces messages on every ingest (snapshot semantics, not delta)', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(
      state,
      appendMessages({ accountId: 'a', messages: [makeMessage({ id: 'm1' })] })
    );
    state = reducer(
      state,
      appendMessages({ accountId: 'a', messages: [makeMessage({ id: 'm2' })] })
    );
    expect(state.messages.a.map(m => m.id)).toEqual(['m2']);
  });

  it('caps the stored message list at 200 entries', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    const many = Array.from({ length: 250 }, (_v, i) => makeMessage({ id: `m-${i}` }));
    state = reducer(state, appendMessages({ accountId: 'a', messages: many }));
    expect(state.messages.a).toHaveLength(200);
    expect(state.messages.a[0].id).toBe('m-0');
    expect(state.messages.a[199].id).toBe('m-199');
  });

  it('updates unread when provided and ignores unknown accounts', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(
      state,
      appendMessages({ accountId: 'a', messages: [makeMessage()], unread: 7 })
    );
    expect(state.unread.a).toBe(7);

    const unchanged = reducer(
      state,
      appendMessages({ accountId: 'ghost', messages: [makeMessage()] })
    );
    expect(unchanged.messages.ghost).toBeUndefined();
  });
});

describe('accountsSlice appendLog', () => {
  it('caps the log buffer at 100 entries and keeps the latest', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    for (let i = 0; i < 120; i += 1) {
      state = reducer(
        state,
        appendLog({
          accountId: 'a',
          entry: { ts: i, level: 'info', msg: `line-${i}` } as AccountLogEntry,
        })
      );
    }
    expect(state.logs.a).toHaveLength(100);
    expect((state.logs.a[0] as AccountLogEntry & { msg: string }).msg).toBe('line-20');
    expect((state.logs.a[99] as AccountLogEntry & { msg: string }).msg).toBe('line-119');
  });
});

describe('accountsSlice resetAccountsState', () => {
  it('returns to the initial empty state', () => {
    let state = reducer(undefined, addAccount(makeAccount({ id: 'a' })));
    state = reducer(state, addAccount(makeAccount({ id: 'b' })));
    state = reducer(state, resetAccountsState());
    expect(state.accounts).toEqual({});
    expect(state.order).toEqual([]);
    expect(state.activeAccountId).toBeNull();
    expect(state.messages).toEqual({});
    expect(state.unread).toEqual({});
    expect(state.logs).toEqual({});
  });
});
