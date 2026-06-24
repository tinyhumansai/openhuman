import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SidebarHeader from './SidebarHeader';

const mockNavigate = vi.fn();
const mockHome = vi.fn();
const mockHide = vi.fn();

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});
vi.mock('./useHomeNav', () => ({ useHomeNav: () => mockHome }));
vi.mock('./RootShellLayout', () => ({ useRootSidebar: () => ({ hide: mockHide }) }));
// Return i18n keys verbatim so queries don't depend on locale.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('SidebarHeader', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders Home, Wallet, Settings, and Collapse buttons', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'nav.home' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'nav.wallet' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'nav.settings' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'chat.hideSidebar' })).toBeInTheDocument();
  });

  it('wallet button navigates to /settings/wallet-balances', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.wallet' }));
    // Carries the backgroundLocation so the desktop Settings modal renders over
    // the page it was opened from.
    expect(mockNavigate).toHaveBeenCalledWith('/settings/wallet-balances', {
      state: { backgroundLocation: expect.objectContaining({ pathname: '/home' }) },
    });
  });

  it('wallet button has correct data-analytics-id', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'nav.wallet' })).toHaveAttribute(
      'data-analytics-id',
      'sidebar-header-wallet'
    );
  });

  it('wallet button has matching aria-label and title', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    const btn = screen.getByRole('button', { name: 'nav.wallet' });
    expect(btn).toHaveAttribute('aria-label', 'nav.wallet');
    // The styled <Tooltip> wrapper re-applies a native `title` fallback so the
    // label still surfaces if the portal pill is occluded by a CEF webview.
    expect(btn).toHaveAttribute('title', 'nav.wallet');
  });

  it('settings button navigates to /settings', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.settings' }));
    expect(mockNavigate).toHaveBeenCalledWith('/settings', {
      state: { backgroundLocation: expect.objectContaining({ pathname: '/home' }) },
    });
  });

  it('Home button invokes the shared Home action', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.home' }));
    expect(mockHome).toHaveBeenCalledTimes(1);
  });

  it('Collapse button calls hide()', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'chat.hideSidebar' }));
    expect(mockHide).toHaveBeenCalledTimes(1);
  });
});
