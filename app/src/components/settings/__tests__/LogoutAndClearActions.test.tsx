import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import LogoutAndClearActions from '../LogoutAndClearActions';

const { mockClearSession, mockClearAllAppData } = vi.hoisted(() => ({
  mockClearSession: vi.fn(),
  mockClearAllAppData: vi.fn(),
}));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    clearSession: mockClearSession,
    snapshot: { auth: { userId: null }, currentUser: null },
  }),
}));

vi.mock('../../../utils/clearAllAppData', () => ({
  clearAllAppData: (...args: unknown[]) => mockClearAllAppData(...args),
}));

function renderActions() {
  return renderWithProviders(<LogoutAndClearActions />, {
    preloadedState: { locale: { current: 'en' } },
  });
}

describe('LogoutAndClearActions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockClearSession.mockReset().mockResolvedValue(undefined);
    mockClearAllAppData.mockReset().mockResolvedValue(undefined);
  });

  it('renders log out as a row and clearing data as a separate destructive zone', () => {
    renderActions();
    // The two used to be adjacent rows with the same amber, weight and icon.
    // Clearing data is now its own zone with its own button, so the assertion
    // checks the split rather than just that both strings exist.
    expect(screen.getByText('Log out')).toBeInTheDocument();
    expect(screen.getByText('Clear app data')).toBeInTheDocument();
    expect(screen.getByTestId('account-destructive-zone')).toBeInTheDocument();
    expect(screen.getByTestId('settings-nav-logout-and-clear')).toHaveTextContent('Clear data');
  });

  it('passes the current snapshot user id + clearSession to clearAllAppData', async () => {
    const user = userEvent.setup();
    renderActions();

    await user.click(screen.getByTestId('settings-nav-logout-and-clear'));
    const confirmButtons = screen.getAllByRole('button', { name: /Clear App Data/i });
    await user.click(confirmButtons[confirmButtons.length - 1]);

    expect(mockClearAllAppData).toHaveBeenCalledTimes(1);
    const args = mockClearAllAppData.mock.calls[0][0];
    expect(args).toMatchObject({ userId: null });
    expect(typeof args.clearSession).toBe('function');
  });

  it('surfaces the core error message when clearAllAppData fails (Windows file-lock guidance)', async () => {
    const user = userEvent.setup();
    mockClearAllAppData.mockRejectedValueOnce(
      new Error(
        'Failed to remove C:\\Users\\me\\.openhuman because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again.'
      )
    );
    renderActions();

    await user.click(screen.getByTestId('settings-nav-logout-and-clear'));
    const confirmButtons = screen.getAllByRole('button', { name: /Clear App Data/i });
    await user.click(confirmButtons[confirmButtons.length - 1]);

    expect(
      await screen.findByText(/locked by another OpenHuman window or process/)
    ).toBeInTheDocument();
  });

  it('falls back to the translated message when the error has no message', async () => {
    const user = userEvent.setup();
    mockClearAllAppData.mockRejectedValueOnce(new Error(''));
    renderActions();

    await user.click(screen.getByTestId('settings-nav-logout-and-clear'));
    const confirmButtons = screen.getAllByRole('button', { name: /Clear App Data/i });
    await user.click(confirmButtons[confirmButtons.length - 1]);

    expect(await screen.findByText(/Failed to clear data and logout/)).toBeInTheDocument();
  });

  it('surfaces logout failures inline next to the Log out row', async () => {
    const user = userEvent.setup();
    mockClearSession.mockRejectedValueOnce(new Error('backend unreachable'));
    renderActions();

    await user.click(screen.getByText('Log out').closest('button')!);

    const alert = await screen.findByTestId('logout-error');
    expect(alert).toHaveTextContent(/sign-in failed|failed to log out|/i); // tolerant
    expect(alert).toBeVisible();
  });
});
