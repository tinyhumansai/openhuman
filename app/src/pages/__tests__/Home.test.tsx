import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { resolveHomeUserName } from '../Home';

vi.mock('../../components/ConnectionIndicator', () => ({
  default: () => <div>Connection Indicator</div>,
}));

vi.mock('../../hooks/useUser', () => ({ useUser: () => ({ user: { firstName: 'Shrey' } }) }));

vi.mock('../../utils/config', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/config')>();
  return { ...actual, APP_VERSION: '0.0.0-test' };
});

vi.mock('react-router-dom', () => ({ useNavigate: () => vi.fn() }));

vi.mock('../../hooks/useUsageState', () => ({
  useUsageState: () => ({ isRateLimited: false, shouldShowBudgetCompletedMessage: false }),
}));

// Default: return 'ok' so most tests see the normal state.
const useAppSelectorMock = vi.fn(() => 'ok' as string);
vi.mock('../../store/hooks', () => ({
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  useAppSelector: (_selector: unknown) => useAppSelectorMock(),
}));

vi.mock('../../store/socketSelectors', () => ({ selectSocketStatus: vi.fn() }));
vi.mock('../../store/connectivitySelectors', () => ({ selectBlockingState: vi.fn() }));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

// Mock restartCoreProcess — default resolves; can be overridden per test.
const restartCoreProcessMock = vi.fn<() => Promise<void>>();
vi.mock('../../services/coreProcessControl', () => ({
  restartCoreProcess: () => restartCoreProcessMock(),
}));

const mockShouldShowBanner = vi.fn<() => boolean>(() => true);
const mockDismissBanner = vi.fn<(id: string) => void>();

vi.mock('../../components/upsell/upsellDismissState', () => ({
  shouldShowBanner: (...args: Parameters<typeof mockShouldShowBanner>) =>
    mockShouldShowBanner(...args),
  dismissBanner: (...args: Parameters<typeof mockDismissBanner>) => mockDismissBanner(...args),
}));

describe('resolveHomeUserName', () => {
  it('uses camelCase name fields when present', () => {
    expect(resolveHomeUserName({ firstName: 'Ada', lastName: 'Lovelace' })).toBe('Ada Lovelace');
  });

  it('falls back to snake_case name fields from core snapshot payloads', () => {
    expect(resolveHomeUserName({ first_name: 'Ada', last_name: 'Lovelace' })).toBe('Ada Lovelace');
  });

  it('falls back to username when no name fields are present', () => {
    expect(resolveHomeUserName({ username: 'openhuman' })).toBe('@openhuman');
  });

  it('falls back to the email local-part when no explicit name exists', () => {
    expect(resolveHomeUserName({ email: 'ada@example.com' })).toBe('ada');
  });

  it('returns User when given null', () => {
    expect(resolveHomeUserName(null)).toBe('User');
  });

  it('returns User when given undefined', () => {
    expect(resolveHomeUserName(undefined)).toBe('User');
  });

  it('returns User when given an empty object', () => {
    expect(resolveHomeUserName({})).toBe('User');
  });

  it('prefixes @-less usernames with @', () => {
    expect(resolveHomeUserName({ username: '@already' })).toBe('@already');
  });

  it('returns User when email local-part is empty', () => {
    expect(resolveHomeUserName({ email: '@nodomain.com' })).toBe('User');
  });
});

describe('Home page — handleRestartCore and blocking state rendering', () => {
  it('shows "Restart Core" button when blocking=core-unreachable (lines 194, 200)', async () => {
    useAppSelectorMock.mockReturnValue('core-unreachable');
    mockShouldShowBanner.mockReturnValue(false);
    const { default: Home } = await import('../Home');
    render(<Home />);

    expect(screen.getByRole('button', { name: /Restart Core/i })).toBeInTheDocument();
  });

  it('does NOT show "Restart Core" button when blocking=ok (line 194)', async () => {
    useAppSelectorMock.mockReturnValue('ok');
    mockShouldShowBanner.mockReturnValue(false);
    const { default: Home } = await import('../Home');
    render(<Home />);

    expect(screen.queryByRole('button', { name: /Restart Core/i })).not.toBeInTheDocument();
  });

  it('handleRestartCore calls restartCoreProcess and resets state on success (lines 78-81, 85)', async () => {
    useAppSelectorMock.mockReturnValue('core-unreachable');
    mockShouldShowBanner.mockReturnValue(false);
    restartCoreProcessMock.mockResolvedValueOnce(undefined);

    const { default: Home } = await import('../Home');
    render(<Home />);

    const btn = screen.getByRole('button', { name: /Restart Core/i });
    fireEvent.click(btn);

    // While waiting, the button should be in "Restarting core…" state.
    expect(screen.getByRole('button', { name: /Restarting core/i })).toBeInTheDocument();

    // After promise resolves the button label reverts.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Restart Core$/i })).toBeInTheDocument()
    );
    expect(restartCoreProcessMock).toHaveBeenCalledTimes(1);
  });

  it('handleRestartCore shows error message when restartCoreProcess throws (lines 78-83, 202)', async () => {
    useAppSelectorMock.mockReturnValue('core-unreachable');
    mockShouldShowBanner.mockReturnValue(false);
    restartCoreProcessMock.mockRejectedValueOnce(new Error('sidecar not found'));

    const { default: Home } = await import('../Home');
    render(<Home />);

    const btn = screen.getByRole('button', { name: /Restart Core/i });
    fireEvent.click(btn);

    await waitFor(() => expect(screen.getByText(/sidecar not found/i)).toBeInTheDocument());
  });

  it('handleRestartCore shows string error when restartCoreProcess throws a non-Error (lines 83)', async () => {
    useAppSelectorMock.mockReturnValue('core-unreachable');
    mockShouldShowBanner.mockReturnValue(false);
    restartCoreProcessMock.mockRejectedValueOnce('raw string error');

    const { default: Home } = await import('../Home');
    render(<Home />);

    const btn = screen.getByRole('button', { name: /Restart Core/i });
    fireEvent.click(btn);

    await waitFor(() => expect(screen.getByText(/raw string error/i)).toBeInTheDocument());
  });
});

describe('Home page — EarlyBirdy banner integration', () => {
  it('shows the EarlyBirdy banner when shouldShowBanner returns true', async () => {
    mockShouldShowBanner.mockReturnValue(true);
    const { default: Home } = await import('../Home');
    render(<Home />);
    expect(screen.getByText('The first 1,000 users get 60% off.')).toBeInTheDocument();
  });

  it('hides the EarlyBirdy banner when shouldShowBanner returns false', async () => {
    mockShouldShowBanner.mockReturnValue(false);
    const { default: Home } = await import('../Home');
    render(<Home />);
    expect(screen.queryByText('The first 1,000 users get 60% off.')).not.toBeInTheDocument();
  });

  it('dismisses the EarlyBirdy banner and calls dismissBanner when the X button is clicked', async () => {
    mockShouldShowBanner.mockReturnValue(true);
    const { default: Home } = await import('../Home');
    render(<Home />);

    const dismissBtn = screen.getByRole('button', { name: /dismiss early bird banner/i });
    fireEvent.click(dismissBtn);

    expect(mockDismissBanner).toHaveBeenCalledWith('home-earlybirdy');
    expect(screen.queryByText('The first 1,000 users get 60% off.')).not.toBeInTheDocument();
  });
});
