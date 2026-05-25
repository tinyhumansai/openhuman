/**
 * Tests for BottomTabBar — verifies that:
 *  - the tab bar renders when the user has a session token and is on a non-hidden path
 *  - the walkthroughAttr mapping (line 222) is exercised by rendering the tabs
 *  - the tab bar is hidden on '/' and '/login' paths
 *
 * [#1123] Covers the walkthroughAttr object added for the Joyride walkthrough.
 */
import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import accountsReducer from '../../store/accountsSlice';
import companionReducer from '../../store/companionSlice';
import notificationReducer from '../../store/notificationSlice';
import BottomTabBar from '../BottomTabBar';

// ── Module-level mocks ─────────────────────────────────────────────────────

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../utils/config', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/config')>();
  return { ...actual, APP_ENVIRONMENT: 'development' };
});

vi.mock('../../utils/accountsFullscreen', () => ({ isAccountsFullscreen: vi.fn(() => false) }));

// ── Helpers ────────────────────────────────────────────────────────────────

interface BuildStoreOpts {
  companionSessionActive?: boolean;
}

function buildStore(opts: BuildStoreOpts = {}) {
  const store = configureStore({
    reducer: {
      accounts: accountsReducer,
      notifications: notificationReducer,
      companion: companionReducer,
    },
  });
  if (opts.companionSessionActive) {
    store.dispatch({
      type: 'companion/setSessionActive',
      payload: { active: true, sessionId: 'sess-test' },
    });
  }
  return store;
}

interface RenderOpts {
  hasToken?: boolean;
  companionSessionActive?: boolean;
  tokenValue?: string;
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
      currentUser: null,
      onboardingCompleted: true,
      chatOnboardingCompleted: true,
      analyticsEnabled: false,
      localState: { encryptionKey: null, onboardingTasks: null },
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

  const store = buildStore({ companionSessionActive: resolved.companionSessionActive });
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
  });

  // [#1123] Covers line 222 — walkthroughAttr object created per-tab inside .map()
  it('renders navigation tabs with data-walkthrough attributes when session is active', async () => {
    await renderBottomTabBar('/home');

    // The Home tab is always visible and has no walkthrough attr (not in the map)
    expect(screen.getByRole('button', { name: 'Home' })).toBeInTheDocument();

    // Chat tab has data-walkthrough="tab-chat" (from walkthroughAttr map)
    const chatBtn = screen.getByRole('button', { name: 'Chat' });
    expect(chatBtn).toBeInTheDocument();
    expect(chatBtn).toHaveAttribute('data-walkthrough', 'tab-chat');
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

  it('still shows the Rewards tab for local sessions', async () => {
    await renderBottomTabBar('/home', { tokenValue: 'header.payload.local' });
    expect(screen.getByRole('button', { name: 'Rewards' })).toBeInTheDocument();
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
});
