import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { registry } from '../../../lib/commands/registry';
import { renderWithProviders } from '../../../test/test-utils';
import SidebarHeader from './SidebarHeader';

const mockNavigate = vi.fn();
const mockHide = vi.fn();
const mockIsWindowsDesktop = vi.fn(() => false);
vi.mock('./WindowsWindowControls', () => ({ isWindowsDesktop: () => mockIsWindowsDesktop() }));

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});
vi.mock('./RootShellLayout', () => ({ useRootSidebar: () => ({ hide: mockHide }) }));
const openUrl = vi.fn().mockResolvedValue(undefined);
vi.mock('../../../utils/openUrl', () => ({ openUrl: (...args: unknown[]) => openUrl(...args) }));
// Return i18n keys verbatim so queries don't depend on locale.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('SidebarHeader', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsWindowsDesktop.mockReturnValue(false);
  });

  it('centers the utility icons on Windows', () => {
    mockIsWindowsDesktop.mockReturnValue(true);
    const { container } = renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(container.querySelector('[data-tauri-drag-region]')).toHaveClass('justify-center');
  });

  it('renders Discord, Search, Settings, and Collapse buttons', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'nav.discord' })).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'shortcuts.action.commandPalette' })
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'nav.settings' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'chat.hideSidebar' })).toBeInTheDocument();
    // The wallet shortcut was removed long ago; Home followed it, since the
    // primary nav directly below already carries Chat.
    expect(screen.queryByRole('button', { name: 'nav.wallet' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'nav.home' })).not.toBeInTheDocument();
  });

  it('Discord button opens the community invite in the browser', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.discord' }));
    expect(openUrl).toHaveBeenCalledWith('https://guild.tinyhumans.ai');
  });

  it('Discord button has correct data-analytics-id', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'nav.discord' })).toHaveAttribute(
      'data-analytics-id',
      'sidebar-header-discord'
    );
  });

  it('Discord button has matching aria-label and title', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    const btn = screen.getByRole('button', { name: 'nav.discord' });
    expect(btn).toHaveAttribute('aria-label', 'nav.discord');
    // The styled <Tooltip> wrapper re-applies a native `title` fallback so the
    // label still surfaces if the portal pill is occluded by a CEF webview.
    expect(btn).toHaveAttribute('title', 'nav.discord');
  });

  it('settings button navigates to /settings', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.settings' }));
    expect(mockNavigate).toHaveBeenCalledWith('/settings');
  });

  // This slot used to be Share Feedback, navigating to `/feedback`. That page
  // is a settings panel now, and the slot opens the command palette.
  it('search button runs the command-palette action', () => {
    const runAction = vi.spyOn(registry, 'runAction');
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'shortcuts.action.commandPalette' }));
    expect(runAction).toHaveBeenCalledWith('meta.command-palette');
  });

  it('no longer renders a feedback button', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    expect(screen.queryByRole('button', { name: 'nav.feedback' })).not.toBeInTheDocument();
  });

  // The band is window chrome. `drag.js` drags a bare region only on a direct
  // hit (`el === composedPath[0]`), so with the icon row nested inside, the
  // row's own box did not drag; `"deep"` covers the subtree. Assert the value
  // and the nesting — presence alone passed before the fix.
  it('marks the header band as a deep drag region around the icon row', () => {
    const { container } = renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    const region = container.querySelector('[data-tauri-drag-region]') as HTMLElement;
    expect(region.getAttribute('data-tauri-drag-region')).toBe('deep');
    expect(region).toHaveClass('pt-[7px]');
    // The icons sit inside the region and keep their clicks: `isDragRegion`
    // short-circuits on a clickable element before it reaches `deep`, and
    // resolving them by button role is what asserts they are still clickable.
    expect(region).toContainElement(screen.getByRole('button', { name: 'nav.settings' }));
    expect(region).toContainElement(screen.getByRole('button', { name: 'chat.hideSidebar' }));
  });

  it('Collapse button calls hide()', () => {
    renderWithProviders(<SidebarHeader />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'chat.hideSidebar' }));
    expect(mockHide).toHaveBeenCalledTimes(1);
  });
});
