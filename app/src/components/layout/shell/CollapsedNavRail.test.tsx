import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { registry } from '../../../lib/commands/registry';
import { renderWithProviders } from '../../../test/test-utils';
import CollapsedNavRail from './CollapsedNavRail';

const mockNavigate = vi.fn();
const mockHome = vi.fn();

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});
vi.mock('./useHomeNav', () => ({ useHomeNav: () => mockHome }));
// Deterministic labels: render the i18n key so queries don't depend on locale.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

describe('CollapsedNavRail', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders Home, Keyboard Shortcuts, and every primary nav destination as icon buttons', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    for (const key of [
      'nav.home',
      'shortcuts.title',
      'nav.chat',
      'nav.brain',
      'nav.flows',
      'nav.connections',
    ]) {
      expect(screen.getByRole('button', { name: key })).toBeInTheDocument();
    }
    // The wallet shortcut was removed from the rail.
    expect(screen.queryByRole('button', { name: 'nav.wallet' })).not.toBeInTheDocument();
    // Human is reached from the chat composer's idle button, not a nav row.
    expect(screen.queryByRole('button', { name: 'nav.human' })).not.toBeInTheDocument();
    // Rewards is cloud-gated; this store has no resolved cloud session.
    expect(screen.queryByRole('button', { name: 'nav.rewards' })).not.toBeInTheDocument();
  });

  it('renders rail icons as sidebar menu primitives', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/connections'] });
    const connections = screen.getByRole('button', { name: 'nav.connections' });
    expect(connections.dataset.slot).toBe('sidebar-menu-button');
    expect(connections.dataset.active).toBe('true');
    expect(connections.closest('[data-slot="sidebar-menu-item"]')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'nav.chat' }).dataset.active).toBe('false');
  });

  it('shortcuts button opens the keyboard-shortcuts help directory', () => {
    const runAction = vi.spyOn(registry, 'runAction').mockReturnValue(true);
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'shortcuts.title' }));
    expect(runAction).toHaveBeenCalledWith('meta.keyboard-shortcuts');
    runAction.mockRestore();
  });

  it('shortcuts button has correct data-analytics-id', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    expect(screen.getByRole('button', { name: 'shortcuts.title' })).toHaveAttribute(
      'data-analytics-id',
      'collapsed-rail-shortcuts'
    );
  });

  it('navigates to a destination path when its icon is clicked', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.brain' }));
    expect(mockNavigate).toHaveBeenCalledWith('/brain');
  });

  it('runs the shared Home action when Home is clicked', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.home' }));
    expect(mockHome).toHaveBeenCalledTimes(1);
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('marks the active destination with aria-current', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/connections'] });
    expect(screen.getByRole('button', { name: 'nav.connections' })).toHaveAttribute(
      'aria-current',
      'page'
    );
    expect(screen.getByRole('button', { name: 'nav.chat' })).not.toHaveAttribute('aria-current');
  });

  it('treats /chat as the active Home state', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/chat/abc'] });
    expect(screen.getByRole('button', { name: 'nav.home' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });

  it('marks Workflows active on the /flows list route', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/flows'] });
    expect(screen.getByRole('button', { name: 'nav.flows' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });

  it('marks Workflows active on a nested /flows/* sub-route', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/flows/some-flow-id'] });
    expect(screen.getByRole('button', { name: 'nav.flows' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });

  it('renders a Settings icon that navigates to /settings', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    const settings = screen.getByRole('button', { name: 'nav.settings' });
    expect(settings).toBeInTheDocument();
    fireEvent.click(settings);
    expect(mockNavigate).toHaveBeenCalledWith('/settings');
  });

  it('marks Settings active on /settings routes', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/settings/general'] });
    expect(screen.getByRole('button', { name: 'nav.settings' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });

  it('marks Settings active on the wallet sub-page (wallet rail removed)', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/settings/wallet-balances'] });
    expect(screen.getByRole('button', { name: 'nav.settings' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });
});
