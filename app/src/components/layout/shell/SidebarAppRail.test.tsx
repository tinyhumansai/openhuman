import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SidebarAppRail from './SidebarAppRail';

const mockNavigate = vi.fn();
const mockDispatch = vi.fn();

const mockState = {
  accounts: {
    accounts: {
      'acct-whatsapp': {
        id: 'acct-whatsapp',
        provider: 'whatsapp',
        label: 'WhatsApp',
        createdAt: '2026-01-01T00:00:00.000Z',
        status: 'open',
      },
    },
    order: ['acct-whatsapp'],
    activeAccountId: null,
    unread: {},
  },
};

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
});

function renderRail(route: string) {
  return render(
    <MemoryRouter initialEntries={[route]}>
      <SidebarAppRail />
    </MemoryRouter>
  );
}
