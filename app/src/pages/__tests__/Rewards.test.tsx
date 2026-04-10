import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Rewards from '../Rewards';

const { rewardsApi } = vi.hoisted(() => ({ rewardsApi: { getMyRewards: vi.fn() } }));

vi.mock('../../components/referral/ReferralRewardsSection', () => ({
  default: () => <div>Referral Rewards Section</div>,
}));

vi.mock('../../components/rewards/RewardsCouponSection', () => ({
  default: () => <div>Rewards Coupon Section</div>,
}));

vi.mock('../../hooks/useUser', () => ({
  useUser: () => ({ user: { subscription: { plan: 'FREE', hasActiveSubscription: false } } }),
}));

vi.mock('../../services/api/rewardsApi', () => ({ rewardsApi }));

describe('Rewards page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders backend-backed achievements', async () => {
    rewardsApi.getMyRewards.mockResolvedValueOnce({
      discord: {
        linked: true,
        discordId: 'discord-123',
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'member',
      },
      summary: {
        unlockedCount: 1,
        totalCount: 2,
        assignedDiscordRoleCount: 1,
        plan: 'PRO',
        hasActiveSubscription: true,
      },
      metrics: {
        currentStreakDays: 7,
        longestStreakDays: 7,
        cumulativeTokens: 12000000,
        featuresUsedCount: 2,
        trackedFeaturesCount: 6,
        lastEvaluatedAt: '2026-04-09T00:00:00.000Z',
        lastSyncedAt: '2026-04-09T01:00:00.000Z',
      },
      achievements: [
        {
          id: 'STREAK_7',
          title: '7-Day Streak',
          description: 'Use OpenHuman on seven consecutive active days.',
          actionLabel: 'Keep your streak alive for 7 days',
          unlocked: true,
          progressLabel: 'Unlocked',
          roleId: 'role-streak-7',
          discordRoleStatus: 'assigned',
          creditAmountUsd: null,
        },
      ],
    });

    render(
      <MemoryRouter>
        <Rewards />
      </MemoryRouter>
    );

    expect(screen.getByText('Loading rewards…')).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText('7-Day Streak')).toBeInTheDocument();
    });

    expect(screen.getByText('Assigned in Discord')).toBeInTheDocument();
    expect(screen.getByText('1/2')).toBeInTheDocument();
  });

  it('shows a conservative error state when rewards fail to load', async () => {
    rewardsApi.getMyRewards.mockRejectedValueOnce({ error: 'Backend offline' });

    render(
      <MemoryRouter>
        <Rewards />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Backend offline');
    });

    expect(screen.getByText('Rewards sync pending')).toBeInTheDocument();
    expect(screen.queryByText('Unlocked')).not.toBeInTheDocument();
  });
});
