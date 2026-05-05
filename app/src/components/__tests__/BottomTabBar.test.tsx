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

function buildStore() {
  return configureStore({
    reducer: { accounts: accountsReducer, notifications: notificationReducer },
  });
}

async function renderBottomTabBar(pathname = '/home', hasToken = true) {
  const { useCoreState } = await import('../../providers/CoreStateProvider');
  vi.mocked(useCoreState).mockReturnValue({
    snapshot: {
      sessionToken: hasToken ? 'tok-test' : null,
      auth: { isAuthenticated: true, userId: 'u1', user: null, profileId: null },
      currentUser: null,
      onboardingCompleted: true,
      chatOnboardingCompleted: true,
      analyticsEnabled: false,
      localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
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

  const store = buildStore();
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
    const { container } = await renderBottomTabBar('/home', false);
    expect(container.firstChild).toBeNull();
  });

  it('returns null on the "/" path even with a session token', async () => {
    const { container } = await renderBottomTabBar('/');
    expect(container.firstChild).toBeNull();
  });
});
