import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { registry } from '../../../lib/commands/registry';
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

  it('renders Home, Keyboard Shortcuts, Settings, and Collapse buttons', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'nav.home' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'shortcuts.title' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'nav.settings' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'chat.hideSidebar' })).toBeInTheDocument();
    // The wallet shortcut was removed (replaced by Home, clear of the macOS
    // window controls).
    expect(screen.queryByRole('button', { name: 'nav.wallet' })).not.toBeInTheDocument();
  });

  it('Home button has correct data-analytics-id', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'nav.home' })).toHaveAttribute(
      'data-analytics-id',
      'sidebar-header-home'
    );
  });

  it('shortcuts button opens the keyboard-shortcuts help directory', () => {
    const runAction = vi.spyOn(registry, 'runAction').mockReturnValue(true);
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'shortcuts.title' }));
    expect(runAction).toHaveBeenCalledWith('meta.keyboard-shortcuts');
    runAction.mockRestore();
  });

  it('shortcuts button has correct data-analytics-id', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'shortcuts.title' })).toHaveAttribute(
      'data-analytics-id',
      'sidebar-header-shortcuts'
    );
  });

  it('shortcuts button has matching aria-label and title', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    const btn = screen.getByRole('button', { name: 'shortcuts.title' });
    expect(btn).toHaveAttribute('aria-label', 'shortcuts.title');
    // The styled <Tooltip> wrapper re-applies a native `title` fallback so the
    // label still surfaces if the portal pill is occluded by a CEF webview.
    expect(btn).toHaveAttribute('title', 'shortcuts.title');
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
