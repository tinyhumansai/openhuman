/**
 * Tests for BottomTabBar — verifies that:
 *  - 6 tabs are rendered (no Rewards tab; Human restored), Activity label is present
 *  - Assistant tab is present (was "Chat", id stays 'chat', label now 'Assistant')
 *  - Walkthrough attributes reflect the new ids (tab-connections, tab-activity)
 *  - Avatar menu opens and shows Account / Billing / Rewards / Invites / Wallet
 *  - Clicking an avatar menu item navigates or opens URL
 *  - The bar is hidden on '/' and '/login' paths
 *
 * Human tab restored as a first-class entry (after the IA Phase 6 merge into
 * Assistant); Chat keeps its "Assistant" label.
 */
import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import accountsReducer from '../../store/accountsSlice';
import agentProfileReducer, { setAgentProfilesFromResponse } from '../../store/agentProfileSlice';
import companionReducer from '../../store/companionSlice';
import notificationReducer from '../../store/notificationSlice';
import themeReducer, { setTabBarLabels, type TabBarLabels } from '../../store/themeSlice';
import BottomTabBar from '../BottomTabBar';

// ── Module-level mocks ─────────────────────────────────────────────────────

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

const agentProfilesApiMock = vi.hoisted(() => ({
  list: vi.fn(),
  select: vi.fn(),
  upsert: vi.fn(),
  delete: vi.fn(),
}));

vi.mock('../../services/api/agentProfilesApi', () => ({ agentProfilesApi: agentProfilesApiMock }));

vi.mock('../../utils/config', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/config')>();
  return { ...actual, APP_ENVIRONMENT: 'development' };
});

vi.mock('../../utils/accountsFullscreen', () => ({ isAccountsFullscreen: vi.fn(() => false) }));
vi.mock('../../services/analytics', () => ({ trackEvent: vi.fn() }));

// Mock openUrl so tests don't try to open real URLs
vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

// ── Helpers ────────────────────────────────────────────────────────────────

interface BuildStoreOpts {
  companionSessionActive?: boolean;
  tabBarLabels?: TabBarLabels;
}

const testProfiles = {
  activeProfileId: 'planner',
  profiles: [
    {
      id: 'default',
      name: 'Orchestrator',
      description: 'Default agent',
      agentId: 'orchestrator',
      builtIn: true,
    },
    {
      id: 'planner',
      name: 'Planner',
      description: 'Plans multi-step work',
      agentId: 'planner',
      builtIn: true,
      avatarUrl: 'https://example.com/planner.png',
    },
    {
      id: 'research',
      name: 'Research',
      description: 'Finds and summarizes sources',
      agentId: 'research',
      builtIn: true,
    },
  ],
};

function buildStore(opts: BuildStoreOpts = {}) {
  const store = configureStore({
    reducer: {
      accounts: accountsReducer,
      notifications: notificationReducer,
      companion: companionReducer,
      agentProfiles: agentProfileReducer,
      theme: themeReducer,
    },
  });
  store.dispatch(setAgentProfilesFromResponse(testProfiles));
  if (opts.companionSessionActive) {
    store.dispatch({
      type: 'companion/setSessionActive',
      payload: { active: true, sessionId: 'sess-test' },
    });
  }
  if (opts.tabBarLabels) {
    store.dispatch(setTabBarLabels(opts.tabBarLabels));
  }
  return store;
}

interface RenderOpts {
  hasToken?: boolean;
  companionSessionActive?: boolean;
  tokenValue?: string;
  currentUser?: unknown;
  tabBarLabels?: TabBarLabels;
}

async function renderBottomTabBar(pathname = '/home', opts: RenderOpts | boolean = {}) {
  // Back-compat: previous callsites passed `hasToken` as the 2nd positional arg.
  const resolved: RenderOpts = typeof opts === 'boolean' ? { hasToken: opts } : opts;
  const hasToken = resolved.hasToken ?? true;
  const tokenValue = resolved.tokenValue ?? 'tok-test';
  const { useCoreState } = await import('../../providers/CoreStateProvider');
  vi.mocked(useCoreState).mockReturnValue({
    snapshot: {
      sessionToken: hasToken ? tokenValue : null,
      auth: { isAuthenticated: true, userId: 'u1', user: null, profileId: null },
      currentUser: resolved.currentUser ?? null,
      onboardingCompleted: true,
      chatOnboardingCompleted: true,
      analyticsEnabled: false,
      localState: { encryptionKey: null, onboardingTasks: null, keyringConsent: null },
      keyringStatus: {
        available: true,
        failureReason: null,
        activeMode: 'os_keyring',
        backendName: 'os',
      },
      runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
    },
    isBootstrapping: false,
    isReady: true,
    teams: [],
    teamMembersById: {},
    teamInvitesById: {},
    setOnboardingCompletedFlag: vi.fn(),
    setOnboardingTasks: vi.fn(),
    refreshSnapshot: vi.fn(),
  } as never);

  const store = buildStore({
    companionSessionActive: resolved.companionSessionActive,
    tabBarLabels: resolved.tabBarLabels,
  });
  return render(
    <Provider store={store}>
      <MemoryRouter initialEntries={[pathname]}>
        <BottomTabBar />
      </MemoryRouter>
    </Provider>
  );
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('BottomTabBar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentProfilesApiMock.select.mockResolvedValue(testProfiles);
  });

  it('renders exactly 6 regular tab buttons (Assistant is rendered separately)', async () => {
    await renderBottomTabBar('/home');
    // Query only the regular pill tabs inside <nav>: exclude the avatar button
    // (aria-haspopup) and the special raised Assistant center button (tab-chat).
    const nav = document.querySelector('nav');
    const navButtons = nav?.querySelectorAll(
      'button:not([aria-haspopup]):not([data-walkthrough="tab-chat"])'
    );
    expect(navButtons).toHaveLength(6);
  });

  it('gives every labelled tab a fixed width when labels are always visible', async () => {
    await renderBottomTabBar('/home', { tabBarLabels: 'always' });
    // With the "always show labels" theme setting, each regular tab is given the
    // same fixed width (w-32) and its label is shown with a truncating class so
    // the row stays symmetric — this exercises the `labelsAlwaysVisible` branch.
    const humanBtn = screen.getByRole('button', { name: 'Human' });
    expect(humanBtn).toHaveClass('w-32');
    expect(humanBtn.querySelector('.truncate')).not.toBeNull();
  });

  it('renders the raised Assistant center button with data-walkthrough="tab-chat"', async () => {
    await renderBottomTabBar('/home');
    const assistantBtn = screen.getByRole('button', { name: 'Assistant' });
    expect(assistantBtn).toBeInTheDocument();
    expect(assistantBtn).toHaveAttribute('data-walkthrough', 'tab-chat');
    expect(assistantBtn).toHaveClass('center-fab');
  });

  it('navigates to /chat and tracks the change when the Assistant center button is clicked', async () => {
    const { trackEvent } = await import('../../services/analytics');
    await renderBottomTabBar('/home');

    fireEvent.click(screen.getByRole('button', { name: 'Assistant' }));

    expect(trackEvent).toHaveBeenCalledWith('tab_bar_change', {
      from_tab: 'home',
      to_tab: 'chat',
      from_path: '/home',
      to_path: '/chat',
    });
  });

  it('does NOT render a Rewards tab', async () => {
    await renderBottomTabBar('/home');
    expect(screen.queryByRole('button', { name: 'Rewards' })).toBeNull();
  });

  it('renders the Human tab (restored as a first-class entry)', async () => {
    await renderBottomTabBar('/home');
    const humanBtn = screen.getByRole('button', { name: 'Human' });
    expect(humanBtn).toBeInTheDocument();
    expect(humanBtn).toHaveAttribute('data-walkthrough', 'tab-human');
  });

  it('navigates to /human and tracks the change when the Human tab is clicked', async () => {
    const { trackEvent } = await import('../../services/analytics');
    await renderBottomTabBar('/home');

    fireEvent.click(screen.getByRole('button', { name: 'Human' }));

    expect(trackEvent).toHaveBeenCalledWith('tab_bar_change', {
      from_tab: 'home',
      to_tab: 'human',
      from_path: '/home',
      to_path: '/human',
    });
  });

  it('renders the Activity tab', async () => {
    await renderBottomTabBar('/home');
    expect(screen.getByRole('button', { name: 'Activity' })).toBeInTheDocument();
  });

  it('renders the Brain tab in the regular row with data-walkthrough="tab-brain"', async () => {
    await renderBottomTabBar('/home');
    const brainBtn = screen.getByRole('button', { name: 'Brain' });
    expect(brainBtn).toBeInTheDocument();
    expect(brainBtn).toHaveAttribute('data-walkthrough', 'tab-brain');
    // It's a regular pill tab now, not the raised center FAB.
    expect(brainBtn).not.toHaveClass('center-fab');
  });

  it('renders the Connections tab with data-walkthrough="tab-connections"', async () => {
    await renderBottomTabBar('/home');
    const connectionsBtn = screen.getByRole('button', { name: 'Connections' });
    expect(connectionsBtn).toBeInTheDocument();
    expect(connectionsBtn).toHaveAttribute('data-walkthrough', 'tab-connections');
  });

  it('renders Activity tab with data-walkthrough="tab-activity"', async () => {
    await renderBottomTabBar('/home');
    const activityBtn = screen.getByRole('button', { name: 'Activity' });
    expect(activityBtn).toHaveAttribute('data-walkthrough', 'tab-activity');
  });

  it('renders Settings tab with data-walkthrough="tab-settings"', async () => {
    await renderBottomTabBar('/home');
    const settingsBtn = screen.getByRole('button', { name: 'Settings' });
    expect(settingsBtn).toHaveAttribute('data-walkthrough', 'tab-settings');
  });

  it('returns null when there is no session token', async () => {
    const { container } = await renderBottomTabBar('/home', { hasToken: false });
    expect(container.firstChild).toBeNull();
  });

  it('renders the pulsing companion dot on the Settings tab when a session is active', async () => {
    const { container } = await renderBottomTabBar('/home', { companionSessionActive: true });
    const settingsBtn = screen.getByRole('button', { name: 'Settings' });
    const dot = settingsBtn.querySelector('.animate-pulse.bg-blue-500');
    expect(dot).not.toBeNull();
    // And not on a non-Settings tab.
    const homeBtn = screen.getByRole('button', { name: 'Home' });
    expect(homeBtn.querySelector('.animate-pulse.bg-blue-500')).toBeNull();
    void container;
  });

  it('returns null on the "/" path even with a session token', async () => {
    const { container } = await renderBottomTabBar('/');
    expect(container.firstChild).toBeNull();
  });

  it('uses pointer-events-none on the full-width shell so side areas do not block clicks', async () => {
    const { container } = await renderBottomTabBar('/home');
    const shell = container.firstElementChild;
    expect(shell).toHaveClass('pointer-events-none');
    expect(shell?.querySelector('nav')).toHaveClass('pointer-events-auto');
  });

  it('tracks tab changes when a different (regular row) tab is clicked', async () => {
    const { trackEvent } = await import('../../services/analytics');
    await renderBottomTabBar('/home');

    fireEvent.click(screen.getByRole('button', { name: 'Brain' }));

    expect(trackEvent).toHaveBeenCalledWith('tab_bar_change', {
      from_tab: 'home',
      to_tab: 'brain',
      from_path: '/home',
      to_path: '/brain',
    });
  });

  it('does not track when the active tab is clicked again', async () => {
    const { trackEvent } = await import('../../services/analytics');
    await renderBottomTabBar('/home');

    fireEvent.click(screen.getByRole('button', { name: 'Home' }));

    expect(trackEvent).not.toHaveBeenCalled();
  });

  it('renders the avatar button with the signed-in user initials', async () => {
    await renderBottomTabBar('/home', { currentUser: { firstName: 'Ada', lastName: 'Lovelace' } });

    const avatar = screen.getByRole('button', { name: 'Account' });
    expect(avatar).toHaveTextContent('AL');
  });

  it('falls back to a generic initial when no user is present', async () => {
    await renderBottomTabBar('/home', { currentUser: null });

    expect(screen.getByRole('button', { name: 'Account' })).toHaveTextContent('U');
  });

  it('avatar menu shows Account, Billing, Rewards, Invites, and Wallet items', async () => {
    await renderBottomTabBar('/home');

    fireEvent.click(screen.getByRole('button', { name: 'Account' }));

    const menu = screen.getByRole('menu', { name: 'Account' });
    const menuItems = menu.querySelectorAll('[role="menuitem"]');
    const labels = Array.from(menuItems).map(el => el.textContent?.trim());
    expect(labels).toContain('Account');
    expect(labels).toContain('Billing');
    expect(labels).toContain('Rewards');
    expect(labels).toContain('Invite a friend');
    expect(labels).toContain('Wallet');
  });

  it('clicking Account in avatar menu closes the menu', async () => {
    await renderBottomTabBar('/home');

    fireEvent.click(screen.getByRole('button', { name: 'Account' }));
    expect(screen.getByRole('menu', { name: 'Account' })).toBeInTheDocument();

    const accountItem = screen.getByRole('menuitem', { name: 'Account' });
    fireEvent.click(accountItem);

    // Menu should close after click
    expect(screen.queryByRole('menu', { name: 'Account' })).toBeNull();
  });

  it('avatar menu does not show cloud-only items for local session', async () => {
    // A local session token contains the literal string 'local'
    await renderBottomTabBar('/home', { tokenValue: 'header.payload.local' });

    fireEvent.click(screen.getByRole('button', { name: 'Account' }));

    const menu = screen.getByRole('menu', { name: 'Account' });
    const menuItems = menu.querySelectorAll('[role="menuitem"]');
    const labels = Array.from(menuItems).map(el => el.textContent?.trim());

    // Account and Wallet are always shown
    expect(labels).toContain('Account');
    expect(labels).toContain('Wallet');

    // Cloud-only items should not appear for local sessions
    expect(labels).not.toContain('Billing');
    expect(labels).not.toContain('Rewards');
    expect(labels).not.toContain('Invite a friend');
  });
});
