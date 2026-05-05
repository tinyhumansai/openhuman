import { fireEvent, render, screen } from '@testing-library/react';
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

vi.mock('../../store/hooks', () => ({ useAppSelector: () => 'connected' }));

vi.mock('../../store/socketSelectors', () => ({ selectSocketStatus: vi.fn() }));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

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
