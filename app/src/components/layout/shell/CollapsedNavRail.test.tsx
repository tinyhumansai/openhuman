import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import CollapsedNavRail from './CollapsedNavRail';

const mockNavigate = vi.fn();

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});
// Deterministic labels: render the i18n key so queries don't depend on locale.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

describe('CollapsedNavRail', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders every primary destination with a visible label, without Home or Discord', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    for (const key of ['nav.chat', 'nav.flows', 'nav.connections', 'nav.settings']) {
      expect(screen.getByRole('button', { name: key })).toBeInTheDocument();
      expect(screen.getByText(key)).toBeVisible();
    }
    expect(screen.queryByRole('button', { name: 'nav.home' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'nav.discord' })).not.toBeInTheDocument();
    // The wallet shortcut was removed from the rail.
    expect(screen.queryByRole('button', { name: 'nav.wallet' })).not.toBeInTheDocument();
    // Human is reached from the chat composer's idle button, not a nav row.
    expect(screen.queryByRole('button', { name: 'nav.human' })).not.toBeInTheDocument();
  });

  it('renders rail icons as sidebar menu primitives', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/connections'] });
    const connections = screen.getByRole('button', { name: 'nav.connections' });
    expect(connections.dataset.slot).toBe('sidebar-menu-button');
    expect(connections.dataset.active).toBe('true');
    expect(connections.closest('[data-slot="sidebar-menu-item"]')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'nav.chat' }).dataset.active).toBe('false');
  });

  it('exposes every collapsed icon label as a tooltip fallback', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    for (const key of ['nav.chat', 'nav.flows', 'nav.connections', 'nav.settings']) {
      expect(screen.getByRole('button', { name: key })).toHaveAttribute('title', key);
    }
  });

  it('navigates to a destination path when its icon is clicked', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/home'] });
    fireEvent.click(screen.getByRole('button', { name: 'nav.flows' }));
    expect(mockNavigate).toHaveBeenCalledWith('/flows');
  });

  it('marks the active destination with aria-current', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/connections'] });
    expect(screen.getByRole('button', { name: 'nav.connections' })).toHaveAttribute(
      'aria-current',
      'page'
    );
    expect(screen.getByRole('button', { name: 'nav.chat' })).not.toHaveAttribute('aria-current');
  });

  it('marks Chat active on nested chat routes', () => {
    renderWithProviders(<CollapsedNavRail />, { initialEntries: ['/chat/abc'] });
    expect(screen.getByRole('button', { name: 'nav.chat' })).toHaveAttribute(
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
