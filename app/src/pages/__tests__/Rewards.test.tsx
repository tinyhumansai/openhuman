import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Rewards from '../Rewards';

const { rewardsApi, openUrl } = vi.hoisted(() => ({
  rewardsApi: { getMyRewards: vi.fn(), claimReward: vi.fn() },
  openUrl: vi.fn(),
}));

const coreStateMock = vi.hoisted(() => vi.fn(() => ({ snapshot: { sessionToken: 'jwt-abc' } })));

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: () => coreStateMock() }));

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
    coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'jwt-abc' } });
  });

  it('shows a local-only message and skips rewards fetch for local sessions', () => {
    coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'header.payload.local' } });

    render(
      <MemoryRouter initialEntries={['/rewards?view=main']}>
        <Rewards />
      </MemoryRouter>
    );

    expect(rewardsApi.getMyRewards).not.toHaveBeenCalled();
    expect(
      screen.getByText(
        'Local login does not earn rewards, coupons, or referral credit. To earn rewards, log out and continue by signing in with an OpenHuman account.'
      )
    ).toBeInTheDocument();
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
          rewardTokens: 500000,
          rewardRecurring: false,
        },
      ],
    });

    render(
      <MemoryRouter initialEntries={['/?view=main']}>
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
      <MemoryRouter initialEntries={['/?view=main']}>
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
            rewardTokens: 500000,
            rewardRecurring: false,
          },
        ],
      });

    render(
      <MemoryRouter initialEntries={['/?view=main']}>
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
      <MemoryRouter initialEntries={['/?view=main']}>
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
      <MemoryRouter initialEntries={['/?view=main']}>
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
      <MemoryRouter initialEntries={['/?view=main']}>
        <Rewards />
      </MemoryRouter>
    );

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Join Discord' })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Join Discord' }));

    expect(openUrl).toHaveBeenCalledWith('https://discord.gg/openhuman');
  });

  it('silently refetches after a claim without flipping into the loading or error state', async () => {
    const claimableSnapshot = {
      discord: {
        linked: false,
        discordId: null,
        username: null,
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'not_linked',
      },
      summary: {
        unlockedCount: 1,
        totalCount: 1,
        assignedDiscordRoleCount: 0,
        claimableCount: 1,
        plan: 'FREE',
        hasActiveSubscription: false,
      },
      metrics: {
        currentStreakDays: 7,
        longestStreakDays: 7,
        cumulativeTokens: 0,
        featuresUsedCount: 0,
        trackedFeaturesCount: 6,
        lastEvaluatedAt: null,
        lastSyncedAt: null,
      },
      achievements: [
        {
          id: 'STREAK_7',
          title: '7-Day Streak',
          description: 'Seven consecutive active days.',
          actionLabel: 'Reach a 7-day streak',
          unlocked: true,
          progressLabel: 'Unlocked',
          roleId: null,
          discordRoleStatus: 'not_configured',
          creditAmountUsd: 1.25,
          rewardTokens: 500000,
          rewardRecurring: false,
          claimable: true,
          claimed: false,
          claimedAt: null,
          claimPeriod: null,
        },
      ],
    };
    // Initial load succeeds; the post-claim silent refetch fails — the page must
    // swallow it (no error banner, no blanking), proving the silent path.
    rewardsApi.getMyRewards
      .mockResolvedValueOnce(claimableSnapshot)
      .mockRejectedValueOnce({ error: 'refetch blip' });
    rewardsApi.claimReward.mockResolvedValueOnce({
      reward: 'STREAK_7',
      recurring: false,
      period: null,
      tokens: 500000,
      amountUsd: 1.25,
      alreadyClaimed: false,
      claimedAt: '2026-07-03T00:00:00.000Z',
      newPromoBalanceUsd: 5,
    });

    render(
      <MemoryRouter initialEntries={['/?view=main']}>
        <Rewards />
      </MemoryRouter>
    );

    await waitFor(() => expect(screen.getByText('7-Day Streak')).toBeInTheDocument());
    expect(rewardsApi.getMyRewards).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId('rewards-claim-STREAK_7'));

    await waitFor(() => expect(rewardsApi.claimReward).toHaveBeenCalledWith('STREAK_7'));
    // The claim triggers exactly one silent reconcile fetch.
    await waitFor(() => expect(rewardsApi.getMyRewards).toHaveBeenCalledTimes(2));

    // The silent refetch failed, but the page stayed intact — no error, no blank.
    expect(screen.getByText('7-Day Streak')).toBeInTheDocument();
    expect(screen.queryByTestId('rewards-error')).not.toBeInTheDocument();
  });

  it('refetches the snapshot when an oauth:success event fires', async () => {
    rewardsApi.getMyRewards.mockResolvedValue({
      discord: {
        linked: false,
        discordId: null,
        username: null,
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
        lastEvaluatedAt: null,
        lastSyncedAt: null,
      },
      achievements: [],
    });

    render(
      <MemoryRouter initialEntries={['/?view=main']}>
        <Rewards />
      </MemoryRouter>
    );

    await waitFor(() => expect(rewardsApi.getMyRewards).toHaveBeenCalledTimes(1));

    fireEvent(window, new CustomEvent('oauth:success', { detail: { toolkit: 'discord' } }));

    await waitFor(() => expect(rewardsApi.getMyRewards).toHaveBeenCalledTimes(2));
  });
});
