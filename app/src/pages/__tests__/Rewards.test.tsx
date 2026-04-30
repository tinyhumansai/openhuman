import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Rewards from '../Rewards';

const { rewardsApi, openUrl } = vi.hoisted(() => ({
  rewardsApi: { getMyRewards: vi.fn() },
  openUrl: vi.fn(),
}));

vi.mock('../../components/rewards/RewardsReferralsTab', () => ({
  default: () => <div>Referral Rewards Section</div>,
}));

vi.mock('../../components/rewards/RewardsRedeemTab', () => ({
  default: () => <div>Rewards Coupon Section</div>,
}));

vi.mock('../../hooks/useUser', () => ({
  useUser: () => ({ user: { subscription: { plan: 'FREE', hasActiveSubscription: false } } }),
}));

vi.mock('../../services/api/rewardsApi', () => ({ rewardsApi }));
vi.mock('../../utils/openUrl', () => ({ openUrl }));

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

    expect(screen.queryAllByText('Loading rewards…').length).toBeGreaterThan(0);

    await waitFor(() => {
      expect(screen.getByText('7-Day Streak')).toBeInTheDocument();
    });

    expect(screen.getByText('Joined the server')).toBeInTheDocument();
    expect(screen.getByText('1 of 2 achievements unlocked')).toBeInTheDocument();
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

  it('retries the snapshot fetch when the user clicks Try again', async () => {
    rewardsApi.getMyRewards
      .mockRejectedValueOnce({ error: 'Backend offline' })
      .mockResolvedValueOnce({
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

    await waitFor(() => {
      expect(screen.getByTestId('rewards-error')).toBeInTheDocument();
    });
    expect(rewardsApi.getMyRewards).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId('rewards-retry'));

    await waitFor(() => {
      expect(screen.getByText('7-Day Streak')).toBeInTheDocument();
    });
    expect(screen.queryByTestId('rewards-error')).not.toBeInTheDocument();
    expect(rewardsApi.getMyRewards).toHaveBeenCalledTimes(2);
  });

  it('switches to the referrals tab content', async () => {
    rewardsApi.getMyRewards.mockResolvedValueOnce({
      discord: {
        linked: false,
        discordId: null,
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'not_linked',
      },
      summary: {
        unlockedCount: 0,
        totalCount: 0,
        assignedDiscordRoleCount: 0,
        plan: 'FREE',
        hasActiveSubscription: false,
      },
      metrics: {
        currentStreakDays: 0,
        longestStreakDays: 0,
        cumulativeTokens: 0,
        featuresUsedCount: 0,
        trackedFeaturesCount: 0,
        lastEvaluatedAt: '2026-04-09T00:00:00.000Z',
        lastSyncedAt: '2026-04-09T01:00:00.000Z',
      },
      achievements: [],
    });

    render(
      <MemoryRouter>
        <Rewards />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByRole('tab', { name: 'Referrals' }));

    expect(screen.getByText('Referral Rewards Section')).toBeInTheDocument();
    expect(screen.queryByText('Rewards Coupon Section')).not.toBeInTheDocument();
    expect(screen.queryByText('Earn community roles')).not.toBeInTheDocument();
  });

  it('switches to the redeem tab content', async () => {
    rewardsApi.getMyRewards.mockResolvedValueOnce({
      discord: {
        linked: false,
        discordId: null,
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'not_linked',
      },
      summary: {
        unlockedCount: 0,
        totalCount: 0,
        assignedDiscordRoleCount: 0,
        plan: 'FREE',
        hasActiveSubscription: false,
      },
      metrics: {
        currentStreakDays: 0,
        longestStreakDays: 0,
        cumulativeTokens: 0,
        featuresUsedCount: 0,
        trackedFeaturesCount: 0,
        lastEvaluatedAt: '2026-04-09T00:00:00.000Z',
        lastSyncedAt: '2026-04-09T01:00:00.000Z',
      },
      achievements: [],
    });

    render(
      <MemoryRouter>
        <Rewards />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByRole('tab', { name: 'Redeem' }));

    expect(screen.getByText('Rewards Coupon Section')).toBeInTheDocument();
    expect(screen.queryByText('Referral Rewards Section')).not.toBeInTheDocument();
  });

  it('opens discord invite via shared openUrl helper', async () => {
    rewardsApi.getMyRewards.mockResolvedValueOnce({
      discord: {
        linked: false,
        discordId: null,
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'not_linked',
      },
      summary: {
        unlockedCount: 0,
        totalCount: 0,
        assignedDiscordRoleCount: 0,
        plan: 'FREE',
        hasActiveSubscription: false,
      },
      metrics: {
        currentStreakDays: 0,
        longestStreakDays: 0,
        cumulativeTokens: 0,
        featuresUsedCount: 0,
        trackedFeaturesCount: 0,
        lastEvaluatedAt: '2026-04-09T00:00:00.000Z',
        lastSyncedAt: '2026-04-09T01:00:00.000Z',
      },
      achievements: [],
    });

    render(
      <MemoryRouter>
        <Rewards />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Join Discord' })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Join Discord' }));

    expect(openUrl).toHaveBeenCalledWith('https://discord.gg/openhuman');
  });
});
