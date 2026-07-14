import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { purgeWebviewAccount } from '../../../services/webviewAccountService';
import SidebarAppRail from './SidebarAppRail';

const mockNavigate = vi.fn();
const mockDispatch = vi.fn();

const whatsappAccount = {
  id: 'acct-whatsapp',
  provider: 'whatsapp',
  label: 'WhatsApp',
  createdAt: '2026-01-01T00:00:00.000Z',
  status: 'open',
};

const accountsWith: Record<string, typeof whatsappAccount> = { 'acct-whatsapp': whatsappAccount };

let mockState = {
  accounts: {
    accounts: accountsWith,
    order: ['acct-whatsapp'],
    activeAccountId: null as string | null,
    unread: {} as Record<string, number>,
  },
};

function setAccounts(order: string[]) {
  mockState = {
    accounts: {
      accounts: order.length ? accountsWith : {},
      order,
      activeAccountId: null,
      unread: {},
    },
  };
}

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));
vi.mock('../../../services/webviewAccountService', () => ({ purgeWebviewAccount: vi.fn() }));
vi.mock('../../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (sel: (state: typeof mockState) => unknown) => sel(mockState),
}));

describe('SidebarAppRail', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setAccounts(['acct-whatsapp']);
  });

  it('selects a provider webview without mutating the current route', () => {
    renderRail('/chat/thread-1');

    fireEvent.click(screen.getByRole('button', { name: 'WhatsApp' }));

    expect(mockNavigate).not.toHaveBeenCalled();
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/setActiveAccount',
      payload: 'acct-whatsapp',
    });
  });

  it('does not navigate again when selecting the agent from a thread route', () => {
    renderRail('/chat/thread-1');

    fireEvent.click(screen.getByRole('button', { name: 'accounts.agent' }));

    expect(mockNavigate).not.toHaveBeenCalled();
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/setActiveAccount',
      payload: '__agent__',
    });
  });

  it('shows the "Add apps" label when no provider apps are connected', () => {
    setAccounts([]);
    renderRail('/chat');

    const addButton = screen.getByTestId('accounts-add-button');
    expect(addButton).toHaveTextContent('accounts.addApps');
  });

  it('collapses the add button to an icon once an app is connected', () => {
    setAccounts(['acct-whatsapp']);
    renderRail('/chat');

    const addButton = screen.getByTestId('accounts-add-button');
    expect(addButton).not.toHaveTextContent('accounts.addApps');
    expect(addButton).toHaveAttribute('aria-label', 'accounts.addApps');
  });

  it('drops the account from state synchronously on disconnect, before the async purge (#4695)', () => {
    setAccounts(['acct-whatsapp']);
    // Hold the purge pending so we can prove `removeAccount` was dispatched
    // *before* the purge (which triggers the app re-mount) resolves. The old
    // order (await purge → removeAccount) let the re-mount re-open the
    // just-purged webview because the account was still in the store.
    let resolvePurge: () => void = () => {};
    vi.mocked(purgeWebviewAccount).mockReturnValue(
      new Promise<void>(res => {
        resolvePurge = res;
      })
    );

    renderRail('/chat');

    fireEvent.contextMenu(screen.getByRole('button', { name: 'WhatsApp' }));
    fireEvent.click(screen.getByText('accounts.disconnect'));

    // removeAccount is dispatched while the purge is still pending.
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/removeAccount',
      payload: { accountId: 'acct-whatsapp' },
    });
    expect(purgeWebviewAccount).toHaveBeenCalledWith('acct-whatsapp');

    resolvePurge();
  });

  it('still drops the account and swallows the error when the purge rejects (#4695)', async () => {
    setAccounts(['acct-whatsapp']);
    // A purge failure must not leave the user with a zombie icon: the account is
    // already removed from state before the await, and the rejection is caught
    // and logged (not surfaced) so the disconnect handler resolves cleanly.
    vi.mocked(purgeWebviewAccount).mockRejectedValue(new Error('purge failed'));

    renderRail('/chat');

    fireEvent.contextMenu(screen.getByRole('button', { name: 'WhatsApp' }));
    fireEvent.click(screen.getByText('accounts.disconnect'));

    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/removeAccount',
      payload: { accountId: 'acct-whatsapp' },
    });
    expect(purgeWebviewAccount).toHaveBeenCalledWith('acct-whatsapp');

    // Flush microtasks so the awaited purge rejection reaches the handler's
    // catch (which logs and swallows it) — the disconnect must not throw.
    await new Promise(resolve => setTimeout(resolve, 0));
  });
});

function renderRail(route: string) {
  return render(
    <MemoryRouter initialEntries={[route]}>
      <SidebarAppRail />
    </MemoryRouter>
  );
}
