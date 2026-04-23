import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ReferralRewardsSection from '../ReferralRewardsSection';

const mocks = vi.hoisted(() => ({
  mockUseCoreState: vi.fn(),
  mockUseUser: vi.fn(),
  mockReferralApi: { getStats: vi.fn(), claimReferral: vi.fn() },
}));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => mocks.mockUseCoreState(),
}));

vi.mock('../../../hooks/useUser', () => ({ useUser: () => mocks.mockUseUser() }));

vi.mock('../../../services/api/referralApi', () => ({ referralApi: mocks.mockReferralApi }));

describe('ReferralRewardsSection', () => {
  const refetch = vi.fn();
  const writeText = vi.fn();
  const share = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.mockUseCoreState.mockReturnValue({ snapshot: { sessionToken: 'test-token' } });
    mocks.mockUseUser.mockReturnValue({ user: null, refetch });
    Object.defineProperty(window.navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });
    Object.defineProperty(window.navigator, 'share', {
      value: share,
      configurable: true,
      writable: true,
    });
    writeText.mockResolvedValue(undefined);
    share.mockResolvedValue(undefined);
  });

  it('copies only the referral code and hides the referral link text', async () => {
    mocks.mockReferralApi.getStats.mockResolvedValueOnce({
      referralCode: 'GQ9F7LEV',
      referralLink: 'https://tinyhumans.ai/signup?ref=GQ9F7LEV',
      totals: { totalRewardUsd: 10, pendingCount: 0, convertedCount: 2 },
      referrals: [],
      canApplyReferral: true,
      appliedReferralCode: null,
    });

    render(<ReferralRewardsSection />);

    const copyButton = await screen.findByRole('button', { name: 'Copy code' });
    fireEvent.click(copyButton);

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith('GQ9F7LEV');
    });
    expect(screen.queryByText('Copy link or code')).not.toBeInTheDocument();
    expect(screen.queryByText('https://tinyhumans.ai/signup?ref=GQ9F7LEV')).not.toBeInTheDocument();
  });

  it('shares referral code text only (without referral url)', async () => {
    mocks.mockReferralApi.getStats.mockResolvedValueOnce({
      referralCode: 'GQ9F7LEV',
      referralLink: 'https://tinyhumans.ai/signup?ref=GQ9F7LEV',
      totals: { totalRewardUsd: 10, pendingCount: 0, convertedCount: 2 },
      referrals: [],
      canApplyReferral: true,
      appliedReferralCode: null,
    });

    render(<ReferralRewardsSection />);

    const shareButton = await screen.findByRole('button', { name: 'Share' });
    fireEvent.click(shareButton);

    await waitFor(() => {
      expect(share).toHaveBeenCalledWith({
        title: 'OpenHuman',
        text: [
          'Join me on OpenHuman.',
          'Referral code: GQ9F7LEV',
          'Download OpenHuman: https://github.com/tinyhumansai/openhuman/releases/latest',
        ].join('\n'),
      });
    });
  });
});
