import type { ApiResponse } from '../../types/api';
import type { RewardsAchievement, RewardsSnapshot } from '../../types/rewards';
import { apiClient } from '../apiClient';

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asNumber(value: unknown): number {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return 0;
}

function asStringOrNull(value: unknown): string | null {
  return typeof value === 'string' && value.trim() !== '' ? value : null;
}

function asFiniteNumberOrNull(value: unknown): number | null {
  if (typeof value === 'number') {
    return Number.isFinite(value) ? value : null;
  }

  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

function normalizeAchievement(value: unknown): RewardsAchievement {
  const raw = asRecord(value) ?? {};
  const creditAmountUsd = asFiniteNumberOrNull(raw.creditAmountUsd);

  return {
    id: typeof raw.id === 'string' ? raw.id : '',
    title: typeof raw.title === 'string' ? raw.title : 'Achievement',
    description: typeof raw.description === 'string' ? raw.description : '',
    actionLabel: typeof raw.actionLabel === 'string' ? raw.actionLabel : '',
    unlocked: raw.unlocked === true,
    progressLabel: typeof raw.progressLabel === 'string' ? raw.progressLabel : '',
    roleId: asStringOrNull(raw.roleId),
    discordRoleStatus:
      raw.discordRoleStatus === 'assigned' ||
      raw.discordRoleStatus === 'not_assigned' ||
      raw.discordRoleStatus === 'not_linked' ||
      raw.discordRoleStatus === 'not_in_guild' ||
      raw.discordRoleStatus === 'not_configured' ||
      raw.discordRoleStatus === 'unavailable'
        ? raw.discordRoleStatus
        : 'unavailable',
    creditAmountUsd: creditAmountUsd == null ? null : asNumber(creditAmountUsd),
  };
}

export function normalizeRewardsSnapshot(payload: unknown): RewardsSnapshot {
  const raw = asRecord(payload) ?? {};
  const rawDiscord = asRecord(raw.discord) ?? {};
  const rawSummary = asRecord(raw.summary) ?? {};
  const rawMetrics = asRecord(raw.metrics) ?? {};
  const achievements = Array.isArray(raw.achievements)
    ? raw.achievements.map(normalizeAchievement).filter(achievement => achievement.id)
    : [];

  return {
    discord: {
      linked: rawDiscord.linked === true,
      discordId: asStringOrNull(rawDiscord.discordId),
      inviteUrl: asStringOrNull(rawDiscord.inviteUrl),
      membershipStatus:
        rawDiscord.membershipStatus === 'member' ||
        rawDiscord.membershipStatus === 'not_in_guild' ||
        rawDiscord.membershipStatus === 'not_linked' ||
        rawDiscord.membershipStatus === 'unavailable'
          ? rawDiscord.membershipStatus
          : 'unavailable',
    },
    summary: {
      unlockedCount: asNumber(rawSummary.unlockedCount),
      totalCount: asNumber(rawSummary.totalCount),
      assignedDiscordRoleCount: asNumber(rawSummary.assignedDiscordRoleCount),
      plan:
        rawSummary.plan === 'BASIC' || rawSummary.plan === 'PRO' || rawSummary.plan === 'FREE'
          ? rawSummary.plan
          : 'FREE',
      hasActiveSubscription: rawSummary.hasActiveSubscription === true,
    },
    metrics: {
      currentStreakDays: asNumber(rawMetrics.currentStreakDays),
      longestStreakDays: asNumber(rawMetrics.longestStreakDays),
      cumulativeTokens: asNumber(rawMetrics.cumulativeTokens),
      featuresUsedCount: asNumber(rawMetrics.featuresUsedCount),
      trackedFeaturesCount: asNumber(rawMetrics.trackedFeaturesCount),
      lastEvaluatedAt: asStringOrNull(rawMetrics.lastEvaluatedAt),
      lastSyncedAt: asStringOrNull(rawMetrics.lastSyncedAt),
    },
    achievements,
  };
}

export const rewardsApi = {
  async getMyRewards(): Promise<RewardsSnapshot> {
    const response = await apiClient.get<ApiResponse<unknown>>('/rewards/me');
    if (!response.success) {
      throw {
        success: false,
        error: response.error ?? response.message ?? 'Unable to load rewards',
      };
    }

    console.debug('[rewards] loaded backend snapshot', {
      achievementCount: Array.isArray((response.data as { achievements?: unknown[] })?.achievements)
        ? (response.data as { achievements: unknown[] }).achievements.length
        : 0,
    });
    return normalizeRewardsSnapshot(response.data);
  },
};
