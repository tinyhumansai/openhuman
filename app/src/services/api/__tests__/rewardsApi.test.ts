import { describe, expect, it, vi } from 'vitest';

import { normalizeRewardsSnapshot, rewardsApi } from '../rewardsApi';

vi.mock('../../apiClient', () => ({ apiClient: { get: vi.fn() } }));

describe('normalizeRewardsSnapshot', () => {
  it('normalizes a backend rewards payload', () => {
    const snapshot = normalizeRewardsSnapshot({
      discord: {
        linked: true,
        discordId: 'discord-123',
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
        },
      ],
    });

    expect(snapshot.discord.membershipStatus).toBe('member');
    expect(snapshot.summary.plan).toBe('PRO');
    expect(snapshot.metrics.currentStreakDays).toBe(7);
    expect(snapshot.achievements[0].discordRoleStatus).toBe('assigned');
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
    expect(snapshot.summary.plan).toBe('FREE');
    expect(snapshot.summary.unlockedCount).toBe(2);
    expect(snapshot.achievements[0].discordRoleStatus).toBe('unavailable');
    expect(snapshot.achievements[0].creditAmountUsd).toBeNull();
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

    expect(apiClient.get).toHaveBeenCalledWith('/rewards/me');
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
});
