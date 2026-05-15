import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SettingsHome from '../SettingsHome';

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
    <MemoryRouter>
      <SettingsHome />
    </MemoryRouter>
  );
}

// --- tests ---

describe('SettingsHome', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('section headers', () => {
    it('renders the General section header', () => {
      renderSettingsHome();
      expect(screen.getByText('General')).toBeInTheDocument();
    });

    it('renders the Features & AI section header', () => {
      renderSettingsHome();
      expect(screen.getByText('Features & AI')).toBeInTheDocument();
    });

    it('renders the Billing & Rewards section header', () => {
      renderSettingsHome();
      expect(screen.getByText('Billing & Rewards')).toBeInTheDocument();
    });

    it('renders the Support section header', () => {
      renderSettingsHome();
      expect(screen.getByText('Support')).toBeInTheDocument();
    });

    it('renders the Advanced section header', () => {
      renderSettingsHome();
      expect(screen.getByText('Advanced')).toBeInTheDocument();
    });

    it('renders the Danger Zone section header', () => {
      renderSettingsHome();
      expect(screen.getByText('Danger Zone')).toBeInTheDocument();
    });
  });

  describe('item grouping order', () => {
    it('places Account and Notifications under General', () => {
      renderSettingsHome();
      const generalHeader = screen.getByText('General');
      const accountItem = screen.getByText('Account');
      const notificationsItem = screen.getByText('Notifications');

      // All should appear after the General header in DOM order
      expect(generalHeader.compareDocumentPosition(accountItem)).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
      expect(generalHeader.compareDocumentPosition(notificationsItem)).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
    });

    it('places Features, AI & Models under Features & AI', () => {
      renderSettingsHome();
      const header = screen.getByText('Features & AI');
      expect(header.compareDocumentPosition(screen.getByText('Features'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
      expect(header.compareDocumentPosition(screen.getByText('AI & Models'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
    });

    it('places Billing & Usage and Rewards under Billing & Rewards', () => {
      renderSettingsHome();
      const header = screen.getByText('Billing & Rewards');
      expect(header.compareDocumentPosition(screen.getByText('Billing & Usage'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
      expect(header.compareDocumentPosition(screen.getByText('Rewards'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
    });

    it('places Restart Tour and About under Support', () => {
      renderSettingsHome();
      const header = screen.getByText('Support');
      expect(header.compareDocumentPosition(screen.getByText('Restart Tour'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
      expect(header.compareDocumentPosition(screen.getByText('About'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
    });

    it('places Developer Options under Advanced', () => {
      renderSettingsHome();
      const header = screen.getByText('Advanced');
      expect(header.compareDocumentPosition(screen.getByText('Developer Options'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
    });

    it('places Clear App Data and Log out under Danger Zone', () => {
      renderSettingsHome();
      const header = screen.getByText('Danger Zone');
      expect(header.compareDocumentPosition(screen.getByText('Clear App Data'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
      expect(header.compareDocumentPosition(screen.getByText('Log out'))).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING
      );
    });
  });

  describe('Rewards menu item', () => {
    it('renders the Rewards item', () => {
      renderSettingsHome();
      expect(screen.getByText('Rewards')).toBeInTheDocument();
    });

    it('navigates to /rewards when clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      // The Rewards item description is used to find the right button
      const rewardsButton = screen.getByText('Rewards').closest('button');
      expect(rewardsButton).toBeTruthy();
      await user.click(rewardsButton!);

      expect(mockNavigate).toHaveBeenCalledWith('/rewards');
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

    it('navigates to /home when Restart Tour is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Restart Tour').closest('button')!);
      expect(mockNavigate).toHaveBeenCalledWith('/home');
    });

    it('navigates to features settings when Features is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Features').closest('button')!);
      expect(mockNavigateToSettings).toHaveBeenCalledWith('features');
    });

    it('navigates to ai-models settings when AI & Models is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('AI & Models').closest('button')!);
      expect(mockNavigateToSettings).toHaveBeenCalledWith('ai-models');
    });

    it('opens billing URL when Billing & Usage is clicked', async () => {
      const { openUrl } = await import('../../../utils/openUrl');
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Billing & Usage').closest('button')!);
      expect(openUrl).toHaveBeenCalledWith('https://billing.example.com');
    });

    it('navigates to about settings when About is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('About').closest('button')!);
      expect(mockNavigateToSettings).toHaveBeenCalledWith('about');
    });

    it('navigates to developer-options settings when Developer Options is clicked', async () => {
      const user = userEvent.setup();
      renderSettingsHome();

      await user.click(screen.getByText('Developer Options').closest('button')!);
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
