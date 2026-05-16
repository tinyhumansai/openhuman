import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import localeReducer from '../../../store/localeSlice';
import SettingsHome from '../SettingsHome';

function makeTestStore() {
  return configureStore({ reducer: { locale: localeReducer } });
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

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    clearSession: vi.fn().mockResolvedValue(undefined),
    snapshot: { auth: { userId: null }, currentUser: null },
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

const { mockClearAllAppData } = vi.hoisted(() => ({
  mockClearAllAppData: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../../utils/clearAllAppData', () => ({
  clearAllAppData: (...args: unknown[]) => mockClearAllAppData(...args),
}));

vi.mock('../../walkthrough/AppWalkthrough', () => ({ resetWalkthrough: vi.fn() }));

// --- helpers ---

function renderSettingsHome() {
  return render(
    <Provider store={makeTestStore()}>
      <MemoryRouter>
        <SettingsHome />
      </MemoryRouter>
    </Provider>
  );
}

// --- tests ---

describe('SettingsHome', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('flat menu', () => {
    // Section headers ("General", "Features & AI", "Billing & Rewards",
    // "Support", "Danger Zone") were intentionally removed — the menu is
    // now a single flat list to reduce visual noise.
    it.each(['General', 'Features & AI', 'Billing & Rewards', 'Support', 'Danger Zone'])(
      'does not render section header: %s',
      label => {
        renderSettingsHome();
        expect(screen.queryByText(label)).not.toBeInTheDocument();
      }
    );

    it('renders the core menu items in a single list', () => {
      renderSettingsHome();
      expect(screen.getByText('Account')).toBeInTheDocument();
      expect(screen.getByText('Alerts')).toBeInTheDocument();
      expect(screen.getByText('Notifications')).toBeInTheDocument();
      expect(screen.getByText('Billing & Usage')).toBeInTheDocument();
      expect(screen.getByText('Advanced')).toBeInTheDocument();
      expect(screen.getByText('Clear App Data')).toBeInTheDocument();
      expect(screen.getByText('Log out')).toBeInTheDocument();
    });

    it('no longer renders Features / AI / Rewards / Restart Tour / About on the home screen', () => {
      renderSettingsHome();
      expect(screen.queryByText('Features')).not.toBeInTheDocument();
      expect(screen.queryByText('AI Configuration')).not.toBeInTheDocument();
      expect(screen.queryByText('Rewards')).not.toBeInTheDocument();
      expect(screen.queryByText('Restart Tour')).not.toBeInTheDocument();
      expect(screen.queryByText('About')).not.toBeInTheDocument();
    });
  });

  describe('language selector', () => {
    it('offers Bahasa Indonesia as a display language', () => {
      renderSettingsHome();

      expect(screen.getByRole('option', { name: /Bahasa Indonesia/ })).toHaveValue('id');
    });
  });

  describe('existing navigation items', () => {
    it('navigates to account settings when Account is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Account').closest('button')!);
      expect(mockNavigateToSettings).toHaveBeenCalledWith('account');
    });

    it('navigates to notifications settings when Notifications is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Notifications').closest('button')!);
      expect(mockNavigateToSettings).toHaveBeenCalledWith('notifications');
    });

    it('navigates to /notifications inbox when Alerts is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Alerts').closest('button')!);
      expect(mockNavigate).toHaveBeenCalledWith('/notifications');
    });

    it('opens billing URL when Billing & Usage is clicked', async () => {
      const { openUrl } = await import('../../../utils/openUrl');
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Billing & Usage').closest('button')!);
      expect(openUrl).toHaveBeenCalledWith('https://billing.example.com');
    });

    it('navigates to developer-options when Advanced is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Advanced').closest('button')!);
      expect(mockNavigateToSettings).toHaveBeenCalledWith('developer-options');
    });
  });

  describe('Clear App Data flow', () => {
    beforeEach(() => {
      mockClearAllAppData.mockReset().mockResolvedValue(undefined);
    });

    it('passes the current snapshot user id + clearSession to clearAllAppData', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Clear App Data').closest('button')!);
      // Confirm in the modal
      const confirmButtons = screen.getAllByRole('button', { name: /Clear App Data/i });
      // The last one is the modal confirm button (first is the menu item we just clicked).
      await user.click(confirmButtons[confirmButtons.length - 1]);

      expect(mockClearAllAppData).toHaveBeenCalledTimes(1);
      const args = mockClearAllAppData.mock.calls[0][0];
      expect(args).toMatchObject({ userId: null });
      expect(typeof args.clearSession).toBe('function');
    });
  });
});
