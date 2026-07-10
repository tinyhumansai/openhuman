import { describe, expect, it, vi } from 'vitest';

import { normalizeRewardsApiError, normalizeRewardsSnapshot, rewardsApi } from '../rewardsApi';

vi.mock('../../apiClient', () => ({ apiClient: { get: vi.fn(), post: vi.fn(), delete: vi.fn() } }));

describe('normalizeRewardsSnapshot', () => {
  it('normalizes a backend rewards payload', () => {
    const snapshot = normalizeRewardsSnapshot({
      discord: {
        linked: true,
        discordId: 'discord-123',
        username: 'cooluser',
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'member',
      },
      summary: {
        unlockedCount: 2,
        totalCount: 8,
        assignedDiscordRoleCount: 1,
        plan: 'PRO',
        hasActiveSubscription: true,
      },
      metrics: {
        currentStreakDays: 7,
        longestStreakDays: 10,
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
          rewardRecurring: true,
          claimable: true,
          claimed: false,
          claimedAt: null,
          claimPeriod: '2026-07',
        },
      ],
    });

    expect(snapshot.discord.membershipStatus).toBe('member');
    expect(snapshot.discord.username).toBe('cooluser');
    expect(snapshot.summary.plan).toBe('PRO');
    expect(snapshot.metrics.currentStreakDays).toBe(7);
    expect(snapshot.achievements[0].discordRoleStatus).toBe('assigned');
    expect(snapshot.achievements[0].rewardTokens).toBe(500000);
    expect(snapshot.achievements[0].rewardRecurring).toBe(true);
    expect(snapshot.achievements[0].claimable).toBe(true);
    expect(snapshot.achievements[0].claimed).toBe(false);
    expect(snapshot.achievements[0].claimPeriod).toBe('2026-07');
  });

  it('falls back safely for malformed payloads', () => {
    const snapshot = normalizeRewardsSnapshot({
      discord: { membershipStatus: 'weird' },
      summary: { plan: 'strange', unlockedCount: '2' },
      achievements: [
        { id: 'POWER_10M', discordRoleStatus: 'mystery', creditAmountUsd: 'not-a-number' },
      ],
    });

    expect(snapshot.discord.membershipStatus).toBe('unavailable');
    expect(snapshot.discord.username).toBeNull();
    expect(snapshot.summary.plan).toBe('FREE');
    expect(snapshot.summary.unlockedCount).toBe(2);
    expect(snapshot.achievements[0].discordRoleStatus).toBe('unavailable');
    expect(snapshot.achievements[0].creditAmountUsd).toBeNull();
    // Missing reward fields default safely.
    expect(snapshot.achievements[0].rewardTokens).toBeNull();
    expect(snapshot.achievements[0].rewardRecurring).toBe(false);
    // Missing claim fields default to not-claimable / not-claimed.
    expect(snapshot.achievements[0].claimable).toBe(false);
    expect(snapshot.achievements[0].claimed).toBe(false);
    expect(snapshot.achievements[0].claimedAt).toBeNull();
    expect(snapshot.achievements[0].claimPeriod).toBeNull();
  });
});

describe('rewardsApi.claimReward', () => {
  it('posts the reward type and normalizes the claim result', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      success: true,
      data: {
        reward: 'POWER_10M',
        recurring: false,
        period: null,
        tokens: 2000000,
        amountUsd: 4,
        alreadyClaimed: false,
        claimedAt: '2026-07-03T00:00:00.000Z',
        newPromoBalanceUsd: 9,
      },
    });

    const result = await rewardsApi.claimReward('POWER_10M');

    expect(apiClient.post).toHaveBeenCalledWith(
      '/rewards/claim',
      { rewardType: 'POWER_10M' },
      { timeout: 15000 }
    );
    expect(result.reward).toBe('POWER_10M');
    expect(result.tokens).toBe(2000000);
    expect(result.amountUsd).toBe(4);
    expect(result.alreadyClaimed).toBe(false);
    expect(result.newPromoBalanceUsd).toBe(9);
  });

  it('throws the backend error message when a claim is rejected', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      success: false,
      data: null,
      error: 'This achievement is not unlocked yet, so it cannot be claimed.',
    });

    await expect(rewardsApi.claimReward('POWER_10M')).rejects.toMatchObject({
      success: false,
      error: 'This achievement is not unlocked yet, so it cannot be claimed.',
    });
  });

  it('normalizes a transport failure into a retryable error', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('network error'));

    await expect(rewardsApi.claimReward('POWER_10M')).rejects.toMatchObject({
      success: false,
      error: 'network error',
    });
  });
});

describe('rewardsApi', () => {
  it('loads and normalizes /rewards/me', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.get).mockResolvedValueOnce({
      success: true,
      data: {
        discord: {
          linked: false,
          discordId: null,
          inviteUrl: null,
          membershipStatus: 'not_linked',
        },
        summary: {
          unlockedCount: 0,
          totalCount: 8,
          assignedDiscordRoleCount: 0,
          plan: 'FREE',
          hasActiveSubscription: false,
        },
        metrics: {
          currentStreakDays: 0,
          longestStreakDays: 0,
          cumulativeTokens: 0,
          featuresUsedCount: 0,
          trackedFeaturesCount: 6,
          lastEvaluatedAt: null,
          lastSyncedAt: null,
        },
        achievements: [],
      },
    });

    const snapshot = await rewardsApi.getMyRewards();

    expect(apiClient.get).toHaveBeenCalledWith('/rewards/me', { timeout: 15000 });
    expect(snapshot.discord.membershipStatus).toBe('not_linked');
    expect(snapshot.summary.totalCount).toBe(8);
  });

  it('throws the backend error when /rewards/me reports failure', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.get).mockResolvedValueOnce({
      success: false,
      data: null,
      error: 'Rewards service unavailable',
    });

    await expect(rewardsApi.getMyRewards()).rejects.toMatchObject({
      error: 'Rewards service unavailable',
    });
  });

  it('preserves backend application errors that contain "timeout" without remapping them', async () => {
    // A backend response like { success: false, error: 'Session timeout. Please log in again.' }
    // must reach the caller unchanged — it must NOT be replaced with the generic network-timeout
    // message, because it carries a real signal from the application layer.
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.get).mockResolvedValueOnce({
      success: false,
      data: null,
      error: 'Session timeout. Please log in again.',
    });

    await expect(rewardsApi.getMyRewards()).rejects.toMatchObject({
      success: false,
      error: 'Session timeout. Please log in again.',
    });
  });

  it('normalizes /rewards/me timeouts into a recoverable message', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.get).mockRejectedValueOnce({
      success: false,
      error: 'Request timed out after 15s',
    });

    await expect(rewardsApi.getMyRewards()).rejects.toMatchObject({
      success: false,
      error: 'Rewards sync timed out. Check your connection and try again.',
    });
  });
});

describe('rewardsApi.disconnectDiscord', () => {
  it('resolves when the backend returns success', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.delete).mockResolvedValueOnce({ success: true, data: null });

    await expect(rewardsApi.disconnectDiscord()).resolves.toBeUndefined();
    expect(apiClient.delete).toHaveBeenCalledWith('/rewards/discord', { timeout: 15000 });
  });

  it('throws a normalized error on transport failure', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.delete).mockRejectedValueOnce(new Error('network error'));

    await expect(rewardsApi.disconnectDiscord()).rejects.toMatchObject({
      success: false,
      error: 'network error',
    });
  });

  it('throws a RewardsApiError when the backend reports failure', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.delete).mockResolvedValueOnce({
      success: false,
      data: null,
      error: 'Unable to disconnect Discord',
    });

    await expect(rewardsApi.disconnectDiscord()).rejects.toMatchObject({
      success: false,
      error: 'Unable to disconnect Discord',
    });
  });

  it('falls back to a default message when backend error has no message', async () => {
    const { apiClient } = await import('../../apiClient');
    vi.mocked(apiClient.delete).mockResolvedValueOnce({ success: false, data: null });

    await expect(rewardsApi.disconnectDiscord()).rejects.toMatchObject({
      success: false,
      error: 'Unable to disconnect Discord',
    });
  });
});

describe('normalizeRewardsApiError', () => {
  it('keeps useful backend errors intact', () => {
    expect(normalizeRewardsApiError({ error: 'Rewards service unavailable' })).toEqual({
      success: false,
      error: 'Rewards service unavailable',
    });
  });

  it('maps abort-style timeout errors to a stable retry message', () => {
    expect(normalizeRewardsApiError(new DOMException('Aborted', 'AbortError'))).toEqual({
      success: false,
      error: 'Rewards sync timed out. Check your connection and try again.',
    });
  });
});
