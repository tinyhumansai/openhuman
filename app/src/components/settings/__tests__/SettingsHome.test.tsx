import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../../lib/i18n/I18nContext';
import type { Locale } from '../../../lib/i18n/types';
import localeReducer from '../../../store/localeSlice';
import themeReducer, {
  type AgentMessageViewMode,
  type FontSize,
  type TabBarLabels,
  type ThemeMode,
} from '../../../store/themeSlice';
import SettingsHome from '../SettingsHome';

// `useDeveloperMode` combines IS_DEV || developerMode.  In tests IS_DEV is
// true (Vite test mode), so mock the hook to control the gate explicitly.
const devModeHoisted = vi.hoisted(() => ({ value: false }));
vi.mock('../../../hooks/useDeveloperMode', () => ({
  useDeveloperMode: () => devModeHoisted.value,
}));

function makeTestStore(locale: Locale = 'en', developerMode = false) {
  return configureStore({
    reducer: { locale: localeReducer, theme: themeReducer },
    preloadedState: {
      locale: { current: locale },
      theme: {
        mode: 'system' as ThemeMode,
        tabBarLabels: 'hover' as TabBarLabels,
        fontSize: 'medium' as FontSize,
        agentMessageViewMode: 'bubbles' as AgentMessageViewMode,
        developerMode,
      },
    },
  });
}

// --- hoisted mocks ---

const { mockNavigate, mockNavigateToSettings } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
  mockNavigateToSettings: vi.fn(),
}));

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateToSettings: mockNavigateToSettings }),
}));

const mockClearSession = vi.fn().mockResolvedValue(undefined);
let mockSessionToken: string | null = null;

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    clearSession: mockClearSession,
    snapshot: { auth: { userId: null }, currentUser: null, sessionToken: mockSessionToken },
  }),
}));

vi.mock('../../../store', () => ({ persistor: { purge: vi.fn().mockResolvedValue(undefined) } }));

vi.mock('../../../utils/links', () => ({ BILLING_DASHBOARD_URL: 'https://billing.example.com' }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('../../../utils/tauriCommands', () => ({
  resetOpenHumanDataAndRestartCore: vi.fn().mockResolvedValue(undefined),
  restartApp: vi.fn().mockResolvedValue(undefined),
  scheduleCefProfilePurge: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../walkthrough/AppWalkthrough', () => ({ resetWalkthrough: vi.fn() }));

// --- helpers ---

function renderSettingsHome({ locale = 'en', withI18n = false, developerMode = false } = {}) {
  // Set the mocked hook value before rendering.
  devModeHoisted.value = developerMode;

  const content = withI18n ? (
    <I18nProvider>
      <SettingsHome />
    </I18nProvider>
  ) : (
    <SettingsHome />
  );

  return render(
    <Provider store={makeTestStore(locale as Locale, developerMode)}>
      <MemoryRouter>{content}</MemoryRouter>
    </Provider>
  );
}

// --- tests ---

describe('SettingsHome', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    devModeHoisted.value = false;
  });

  describe('layman groups structure', () => {
    it('renders the merged layman card and the About container', () => {
      renderSettingsHome();
      // The layman groups (Account/Assistant/Privacy/Notifications) merge into a
      // single card with no subheadings; About keeps its own container.
      expect(screen.getByTestId('settings-group-main')).toBeInTheDocument();
      expect(screen.getByTestId('settings-group-about')).toBeInTheDocument();
      expect(screen.queryByTestId('settings-group-account')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-group-assistant')).not.toBeInTheDocument();
    });

    it('renders the Account group items', () => {
      renderSettingsHome();
      // Account group: Account (hub), Language, Appearance, Devices, Data Sync.
      // Team & members and Data & migration moved out (dev / removed).
      expect(screen.getByTestId('settings-nav-profile')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-language')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-appearance')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-devices')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-data-sync')).toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-team')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-migration')).not.toBeInTheDocument();
    });

    it('renders the Assistant group items', () => {
      renderSettingsHome();
      // Only Personality + Face/Mascot stay layman-facing; the rest moved to
      // Developer & Diagnostics.
      expect(screen.getByTestId('settings-nav-persona')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-mascot')).toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-voice')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-permissions')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-activity-level')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-screen-intelligence')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-companion')).not.toBeInTheDocument();
    });

    it('does not surface Privacy/Security/Approvals on the home list', () => {
      renderSettingsHome();
      // Privacy is reached via the Account hub; Security + Approvals live under
      // Developer & Diagnostics. None of them are top-level home rows.
      expect(screen.queryByTestId('settings-nav-privacy')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-security')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-approval-history')).not.toBeInTheDocument();
    });

    it('renders the Notifications group item', () => {
      renderSettingsHome();
      expect(screen.getByTestId('settings-nav-notifications-hub')).toBeInTheDocument();
    });

    it('renders the About item always (even without developer mode)', () => {
      renderSettingsHome({ developerMode: false });
      expect(screen.getByTestId('settings-nav-about')).toBeInTheDocument();
    });

    it('old flat section headers are not rendered', () => {
      // Section headers ("General", "Features & AI", "Billing & Rewards",
      // "Support", "Danger Zone") were removed in Phase 4.
      renderSettingsHome();
      expect(screen.queryByText('Features & AI')).not.toBeInTheDocument();
      expect(screen.queryByText('Support')).not.toBeInTheDocument();
      expect(screen.queryByText('Danger Zone')).not.toBeInTheDocument();
    });
  });

  describe('items no longer on the home screen', () => {
    it('renders Agents and Crypto section hubs on the home screen', () => {
      // Pass A surfaced agents-settings and crypto on the home screen as part of
      // the merged layman card (assistant group + crypto group).
      renderSettingsHome();
      expect(screen.getByTestId('settings-nav-agents-settings')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-crypto')).toBeInTheDocument();
    });

    it('no longer renders Alerts / stand-alone Notifications on the home screen', () => {
      // Notifications now lives in its own Notifications group (notifications-hub).
      renderSettingsHome();
      expect(screen.queryByTestId('settings-nav-alerts')).not.toBeInTheDocument();
      expect(screen.queryByTestId('settings-nav-notifications')).not.toBeInTheDocument();
    });

    it('no longer renders destructive actions on the home screen', () => {
      // Clear App Data + Log out moved to Settings → Account.
      renderSettingsHome();
      expect(screen.queryByText('Clear App Data')).not.toBeInTheDocument();
      expect(screen.queryByText('Log out')).not.toBeInTheDocument();
    });

    it('renders Features and AI section hubs on home; Rewards and Restart Tour remain absent', () => {
      // Pass A moved Features and AI onto the home screen as merged-card entries.
      // Rewards and Restart Tour are not home items (Rewards lives in the avatar
      // menu; Restart Tour is in Developer & Diagnostics only).
      renderSettingsHome();
      expect(screen.getByTestId('settings-nav-features')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-ai')).toBeInTheDocument();
      expect(screen.queryByText('Rewards')).not.toBeInTheDocument();
      expect(screen.queryByText('Restart Tour')).not.toBeInTheDocument();
    });
  });

  describe('language selector', () => {
    it('offers Bahasa Indonesia as a display language', () => {
      renderSettingsHome();
      expect(screen.getByRole('option', { name: /Bahasa Indonesia/ })).toHaveValue('id');
    });
  });

  describe('navigation — layman groups', () => {
    it('navigates to account settings when Profile is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-profile'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('account');
    });

    it('navigates to persona when Personality is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome({ withI18n: true });

      await user.click(screen.getByTestId('settings-nav-persona'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('persona');
    });

    it('navigates to mascot when Face / Mascot is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-mascot'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('mascot');
    });

    it('navigates to notifications-hub when Notifications is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-notifications-hub'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('notifications-hub');
    });

    it('navigates to about when About is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-about'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('about');
    });

    it('does not render Billing & Usage in Settings (billing is in avatar menu)', () => {
      // Per the IA redesign doc, billing/rewards live in the avatar menu — not in Settings.
      renderSettingsHome();
      expect(screen.queryByTestId('settings-nav-billing')).not.toBeInTheDocument();
      expect(screen.queryByText('Billing & Usage')).not.toBeInTheDocument();
    });

    it('navigates to developer-options when "Developer & Diagnostics" is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-developer-options'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('developer-options');
    });
  });

  describe('developer & diagnostics (always visible)', () => {
    it('shows the developer-options entry regardless of the developerMode preference', () => {
      renderSettingsHome({ developerMode: false });
      expect(screen.getByTestId('settings-nav-developer-options')).toBeInTheDocument();
      // useT() resolves to English even without I18nProvider
      expect(screen.getByText('Developer & Diagnostics')).toBeInTheDocument();
    });
  });

  describe('local session gating', () => {
    beforeEach(() => {
      // Use a valid local-session token (three parts, last part = 'local')
      mockSessionToken = 'header.payload.local';
    });

    afterEach(() => {
      mockSessionToken = null;
    });

    it('does not render Billing & Usage in Settings regardless of session type (billing is in avatar menu)', () => {
      // Billing moved to avatar menu per IA redesign — never shown in Settings.
      renderSettingsHome();
      expect(screen.queryByText('Billing & Usage')).not.toBeInTheDocument();
    });

    it('does not render Billing & Usage in Settings even when not in local mode', () => {
      // Billing moved to avatar menu per IA redesign — never shown in Settings.
      mockSessionToken = null;
      renderSettingsHome();
      expect(screen.queryByText('Billing & Usage')).not.toBeInTheDocument();
    });
  });

  describe('i18n — Chinese locale', () => {
    it('localizes Appearance and Mascot menu items', () => {
      renderSettingsHome({ locale: 'zh-CN', withI18n: true });

      expect(screen.getByText('外观')).toBeInTheDocument();
      expect(screen.getByText('选择浅色、深色或跟随系统主题')).toBeInTheDocument();
    });
  });

  // ---------------------------------------------------------------------------
  // Pass A — newly-surfaced section entries
  // ---------------------------------------------------------------------------

  describe('Pass A section hubs', () => {
    it('renders the 5 newly-surfaced section entry hubs in the merged card', () => {
      // Pass A merged AI, Agents, Features, Integrations (composio), and Crypto
      // directly onto the home screen as section-hub entries. All 5 must render
      // as navigable items in the settings-group-main card.
      renderSettingsHome();
      expect(screen.getByTestId('settings-nav-ai')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-agents-settings')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-features')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-composio')).toBeInTheDocument();
      expect(screen.getByTestId('settings-nav-crypto')).toBeInTheDocument();
    });

    it('clicking ai hub navigates to ai', async () => {
      const user = userEvent.setup();
      renderSettingsHome();
      await user.click(screen.getByTestId('settings-nav-ai'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('ai');
    });

    it('clicking agents-settings hub navigates to agents-settings', async () => {
      const user = userEvent.setup();
      renderSettingsHome();
      await user.click(screen.getByTestId('settings-nav-agents-settings'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('agents-settings');
    });

    it('clicking features hub navigates to features', async () => {
      const user = userEvent.setup();
      renderSettingsHome();
      await user.click(screen.getByTestId('settings-nav-features'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('features');
    });

    it('clicking composio hub navigates to composio', async () => {
      const user = userEvent.setup();
      renderSettingsHome();
      await user.click(screen.getByTestId('settings-nav-composio'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('composio');
    });

    it('clicking crypto hub navigates to crypto', async () => {
      const user = userEvent.setup();
      renderSettingsHome();
      await user.click(screen.getByTestId('settings-nav-crypto'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('crypto');
    });
  });

  describe('navigation — account group items (lines 261, 268, 275)', () => {
    it('navigates to devices when Devices is clicked (line 268)', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-devices'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('devices');
    });

    it('navigates to memory-sync when Data Sync is clicked (line 275)', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-data-sync'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('memory-sync');
    });

    it('navigates to appearance when Appearance is clicked (line 261)', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByTestId('settings-nav-appearance'));
      expect(mockNavigateToSettings).toHaveBeenCalledWith('appearance');
    });
  });

  // Clear App Data flow moved to LogoutAndClearActions (rendered on Account
  // page) — see LogoutAndClearActions.test.tsx.
});
