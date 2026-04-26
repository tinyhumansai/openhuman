import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { LATEST_APP_DOWNLOAD_URL } from '../../../utils/config';
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
  const statsFixture = {
    referralCode: 'GQ9F7LEV',
    totals: { totalRewardUsd: 10, pendingCount: 0, convertedCount: 2 },
    referrals: [],
    canApplyReferral: true,
    appliedReferralCode: null,
  };

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

  it('copies only the referral code', async () => {
    mocks.mockReferralApi.getStats.mockResolvedValueOnce(statsFixture);

    render(<ReferralRewardsSection />);

    const copyButton = await screen.findByRole('button', { name: 'Copy code' });
    fireEvent.click(copyButton);

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith('GQ9F7LEV');
    });
  });

  it('shares referral copy without a per-user referral link', async () => {
    mocks.mockReferralApi.getStats.mockResolvedValueOnce(statsFixture);

    render(<ReferralRewardsSection />);

    const shareButton = await screen.findByRole('button', { name: 'Share' });
    fireEvent.click(shareButton);

    await waitFor(() => {
      expect(share).toHaveBeenCalledWith({
        title: 'OpenHuman',
        text: expect.stringContaining(`Download OpenHuman: ${LATEST_APP_DOWNLOAD_URL}`),
      });
    });
    expect(share).not.toHaveBeenCalledWith(
      expect.objectContaining({ text: expect.stringContaining('signup?ref=') })
    );
  });

  it('falls back to clipboard when navigator.share is absent', async () => {
    Object.defineProperty(window.navigator, 'share', { value: undefined, configurable: true });
    mocks.mockReferralApi.getStats.mockResolvedValueOnce(statsFixture);

    render(<ReferralRewardsSection />);

    fireEvent.click(await screen.findByRole('button', { name: 'Share' }));

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining('Referral code: GQ9F7LEV'));
    });
    expect(writeText).toHaveBeenCalledWith(
      expect.not.stringContaining('https://tinyhumans.ai/signup?ref=')
    );
    expect(await screen.findByText('Copied')).toBeInTheDocument();
  });

  it('shows Copy failed hint when clipboard throws', async () => {
    writeText.mockRejectedValueOnce(new Error('NotAllowedError'));
    mocks.mockReferralApi.getStats.mockResolvedValueOnce(statsFixture);

    render(<ReferralRewardsSection />);

    fireEvent.click(await screen.findByRole('button', { name: 'Copy code' }));

    expect(await screen.findByText('Copy failed')).toBeInTheDocument();
  });
});
