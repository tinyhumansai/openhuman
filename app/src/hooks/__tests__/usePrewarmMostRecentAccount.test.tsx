import { renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { prewarmWebviewAccount } from '../../services/webviewAccountService';
import { store } from '../../store';
import {
  addAccount,
  resetAccountsState,
  setActiveAccount,
  setLastActiveAccount,
} from '../../store/accountsSlice';
import type { Account, AccountStatus } from '../../types/accounts';
import { PREWARM_MAX_ACCOUNTS, usePrewarmMostRecentAccount } from '../usePrewarmMostRecentAccount';

vi.mock('../../services/webviewAccountService', () => ({
  prewarmWebviewAccount: vi.fn().mockResolvedValue(undefined),
}));

function makeAccount(
  overrides: Partial<Account> & { id: string; provider: Account['provider'] }
): Account {
  return {
    label: overrides.id,
    createdAt: '2026-01-01T00:00:00Z',
    status: 'closed' as AccountStatus,
    ...overrides,
  };
}

const wrapper = ({ children }: { children: ReactNode }) => (
  <Provider store={store}>{children}</Provider>
);

function seedStore(opts: {
  accounts: Account[];
  activeAccountId: string | null;
  mruAccountId: string | null;
}): void {
  store.dispatch(resetAccountsState());
  for (const acct of opts.accounts) {
    store.dispatch(addAccount(acct));
  }
  store.dispatch(setActiveAccount(opts.activeAccountId));
  store.dispatch(setLastActiveAccount(opts.mruAccountId));
}

function renderPrewarmHook(args: {
  accounts: Account[];
  activeAccountId: string | null;
  mruAccountId: string | null;
}): void {
  seedStore(args);
  const accountsById: Record<string, Account | undefined> = Object.fromEntries(
    args.accounts.map(a => [a.id, a])
  );
  renderHook(
    () =>
      usePrewarmMostRecentAccount({
        accounts: args.accounts,
        accountsById,
        activeAccountId: args.activeAccountId,
      }),
    { wrapper }
  );
}

describe('usePrewarmMostRecentAccount (issue #1233)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    store.dispatch(resetAccountsState());
    vi.restoreAllMocks();
  });

  it('prewarms the MRU account when conditions are met', () => {
    renderPrewarmHook({
      accounts: [makeAccount({ id: 'acct-1', provider: 'slack', status: 'closed' })],
      activeAccountId: null,
      mruAccountId: 'acct-1',
    });
    expect(prewarmWebviewAccount).toHaveBeenCalledTimes(1);
    expect(prewarmWebviewAccount).toHaveBeenCalledWith('acct-1', 'slack');
  });

  it('does nothing when no MRU id is in the store', () => {
    renderPrewarmHook({
      accounts: [makeAccount({ id: 'acct-1', provider: 'slack' })],
      activeAccountId: null,
      mruAccountId: null,
    });
    expect(prewarmWebviewAccount).not.toHaveBeenCalled();
  });

  it('does nothing when the accounts list is empty', () => {
    renderPrewarmHook({ accounts: [], activeAccountId: null, mruAccountId: 'acct-1' });
    expect(prewarmWebviewAccount).not.toHaveBeenCalled();
  });

  it('does nothing when accounts.length exceeds PREWARM_MAX_ACCOUNTS', () => {
    const tooMany: Account[] = Array.from({ length: PREWARM_MAX_ACCOUNTS + 1 }, (_, i) =>
      makeAccount({ id: `acct-${i}`, provider: 'slack', status: 'closed' })
    );
    renderPrewarmHook({ accounts: tooMany, activeAccountId: null, mruAccountId: 'acct-0' });
    expect(prewarmWebviewAccount).not.toHaveBeenCalled();
  });

  it('does nothing when the MRU account is no longer in the store', () => {
    renderPrewarmHook({
      accounts: [makeAccount({ id: 'acct-1', provider: 'telegram' })],
      activeAccountId: null,
      mruAccountId: 'acct-removed',
    });
    expect(prewarmWebviewAccount).not.toHaveBeenCalled();
  });

  it('does nothing when the MRU account is already the active one', () => {
    renderPrewarmHook({
      accounts: [makeAccount({ id: 'acct-1', provider: 'slack' })],
      activeAccountId: 'acct-1',
      mruAccountId: 'acct-1',
    });
    expect(prewarmWebviewAccount).not.toHaveBeenCalled();
  });

  it.each<AccountStatus>(['pending', 'loading', 'open'])(
    'does nothing when the MRU account is already in status %s',
    status => {
      renderPrewarmHook({
        accounts: [makeAccount({ id: 'acct-1', provider: 'slack', status })],
        activeAccountId: null,
        mruAccountId: 'acct-1',
      });
      expect(prewarmWebviewAccount).not.toHaveBeenCalled();
    }
  );

  it('still prewarms when the MRU account is in status timeout', () => {
    renderPrewarmHook({
      accounts: [makeAccount({ id: 'acct-1', provider: 'slack', status: 'timeout' })],
      activeAccountId: null,
      mruAccountId: 'acct-1',
    });
    expect(prewarmWebviewAccount).toHaveBeenCalledWith('acct-1', 'slack');
  });
});
