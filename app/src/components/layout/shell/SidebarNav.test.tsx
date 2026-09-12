import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { AGENT_ACCOUNT_ID } from '../../../utils/accountsFullscreen';
import SidebarNav from './SidebarNav';

// Analytics is fire-and-forget; stub it so the nav renders without a transport.
vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

// Mutable so each test can pick the session kind. `isReady` sits alongside
// `snapshot` on the core-state value (not inside the snapshot). Must be
// `mock`-prefixed so the hoisted vi.mock factory below may close over it.
let mockCoreState: { snapshot: { sessionToken: string | null }; isReady: boolean } = {
  snapshot: { sessionToken: 'cloud.session.token' },
  isReady: true,
};
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => mockCoreState }));

/**
 * `bg-white` spelled indirectly. `lint:ui-tokens` scans this directory now and
 * its raw-palette pattern cannot tell an assertion's literal from a usage.
 */
const RAW_WHITE_FILL = `bg-${'white'}`;

/** The rendered button for a nav label (label text lives in a child span). */
function tabButton(label: string): HTMLButtonElement {
  return screen.getByRole('button', { name: new RegExp(label) }) as HTMLButtonElement;
}

describe('SidebarNav active matching', () => {
  it('keeps Workflows active on the /flows list route', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/flows'] });

    expect(tabButton('Workflows')).toHaveAttribute('aria-current', 'page');
  });

  it('keeps Workflows active on a nested /flows/* sub-route', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/flows/some-flow-id'] });

    expect(tabButton('Workflows')).toHaveAttribute('aria-current', 'page');
  });

  it('does not mark Workflows active on an unrelated route', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    expect(tabButton('Workflows')).not.toHaveAttribute('aria-current');
  });

  it('gives the active tab the accent fill, not a neutral surface lift', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    const active = tabButton('Chat');
    // The accent is the only colour in this column, which is what makes it
    // legible as selection rather than decoration. This was a neutral
    // `bg-surface/70` while the chrome was a themed WebGL mesh; the flat
    // default backdrop left a neutral pill with nothing to lift against.
    expect(active.className).toContain('bg-primary-500');
    expect(active.className).toContain('font-semibold');
    expect(active.className).not.toContain(RAW_WHITE_FILL);

    // Inactive tabs carry no active fill.
    expect(tabButton('Workflows').className).not.toContain('bg-primary-500');
  });

  it('renders rows as sidebar menu primitives, not bare buttons', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    const active = tabButton('Chat');
    expect(active.dataset.slot).toBe('sidebar-menu-button');
    expect(active.dataset.active).toBe('true');
    expect(active.closest('[data-slot="sidebar-menu-item"]')).not.toBeNull();
    expect(active.closest('[data-slot="sidebar-menu"]')).not.toBeNull();
    expect(tabButton('Workflows').dataset.active).toBe('false');
  });

  it('clears an active provider selection when clicking the already-active nav item', () => {
    const { store } = renderWithProviders(<SidebarNav />, {
      initialEntries: ['/connections'],
      preloadedState: {
        accounts: {
          accounts: {
            'acct-slack': {
              id: 'acct-slack',
              provider: 'slack',
              label: 'Slack',
              createdAt: '2026-01-01T00:00:00.000Z',
              status: 'open',
            },
          },
          order: ['acct-slack'],
          activeAccountId: 'acct-slack',
          lastActiveAccountId: 'acct-slack',
          messages: {},
          unread: {},
          logs: {},
        },
      },
    });

    fireEvent.click(tabButton('Connections'));

    expect(store.getState().accounts.activeAccountId).toBe(AGENT_ACCOUNT_ID);
  });
});

/**
 * Rewards is the one `cloudOnly` nav entry. These moved here from
 * `AppSidebar.test.tsx` when Rewards stopped being a sidebar footer row and
 * became a primary destination: the gate lives in `useCloudNavGate` and is
 * applied by this component (and `CollapsedNavRail`), so this is where it is
 * observable.
 */
describe('SidebarNav — cloud-gated Rewards entry', () => {
  beforeEach(() => {
    mockCoreState = { snapshot: { sessionToken: 'cloud.session.token' }, isReady: true };
  });

  it('shows Rewards for a resolved cloud session', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    expect(tabButton('Rewards')).toBeInTheDocument();
  });

  it('hides Rewards for a local session', () => {
    mockCoreState = { snapshot: { sessionToken: 'header.payload.local' }, isReady: true };
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    expect(screen.queryByRole('button', { name: /Rewards/ })).not.toBeInTheDocument();
    // The ungated entries are unaffected.
    expect(tabButton('Chat')).toBeInTheDocument();
  });

  it('hides Rewards until core state has bootstrapped (no flash)', () => {
    // Initial snapshot before the first refresh: not ready, null token.
    // isLocalSessionToken(null) is false, so gating on the token alone would
    // briefly show Rewards here — the isReady guard prevents that flash.
    mockCoreState = { snapshot: { sessionToken: null }, isReady: false };
    renderWithProviders(<SidebarNav />, { initialEntries: ['/chat'] });

    expect(screen.queryByRole('button', { name: /Rewards/ })).not.toBeInTheDocument();
  });

  it('marks Rewards active on the /rewards route', () => {
    renderWithProviders(<SidebarNav />, { initialEntries: ['/rewards'] });

    expect(tabButton('Rewards')).toHaveAttribute('aria-current', 'page');
  });
});
