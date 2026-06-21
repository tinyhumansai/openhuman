import createDebug from 'debug';

import type { ApiError, ApiResponse } from '../../types/api';
import type { RewardsAchievement, RewardsSnapshot } from '../../types/rewards';
import { apiClient } from '../apiClient';

const REWARDS_SNAPSHOT_TIMEOUT_MS = 15_000;
const log = createDebug('rewards:api');

export type RewardsApiError = ApiError & { code?: string; status?: number };

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

export function normalizeRewardsApiError(error: unknown): RewardsApiError {
  const raw = asRecord(error);
  const message =
    (typeof raw?.error === 'string' && raw.error) ||
    (typeof raw?.message === 'string' && raw.message) ||
    (error instanceof Error ? error.message : null) ||
    'Unable to load rewards';
  const code = typeof raw?.code === 'string' ? raw.code : undefined;
  const status = typeof raw?.status === 'number' ? raw.status : undefined;
  const name =
    (typeof raw?.name === 'string' && raw.name) ||
    (error instanceof Error ? error.name : undefined);
  const lowerMessage = message.toLowerCase();
  const isTimeout =
    lowerMessage.includes('timed out') ||
    lowerMessage.includes('timeout') ||
    code === 'ETIMEDOUT' ||
    code === 'ECONNABORTED' ||
    name === 'AbortError';

  if (isTimeout) {
    return {
      success: false,
      error: 'Rewards sync timed out. Check your connection and try again.',
      ...(code ? { code } : {}),
      ...(status == null ? {} : { status }),
    };
  }

  return {
    success: false,
    error: message,
    ...(code ? { code } : {}),
    ...(status == null ? {} : { status }),
  };
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
      username: asStringOrNull(rawDiscord.username),
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
    let response: ApiResponse<unknown>;
    try {
      response = await apiClient.get<ApiResponse<unknown>>('/rewards/me', {
        timeout: REWARDS_SNAPSHOT_TIMEOUT_MS,
      });
    } catch (transportError) {
      // Transport-level failure (network error, timeout, abort) — normalize to
      // a stable retryable message. String-based timeout heuristics are only
      // safe here where the error comes from the HTTP layer, not from backend
      // application logic.
      const normalized = normalizeRewardsApiError(transportError);
      log(
        'snapshot transport failed error=%s code=%s status=%s',
        normalized.error,
        normalized.code ?? 'none',
        normalized.status ?? 'none'
      );
      throw normalized;
    }

    if (!response.success) {
      // Backend application error — preserve the exact message so callers see
      // the real signal (e.g. "Session timeout. Please log in again." must not
      // be remapped to the generic network-timeout message).
      const appError: RewardsApiError = {
        success: false,
        error: response.error ?? response.message ?? 'Unable to load rewards',
      };
      log('snapshot backend error error=%s', appError.error);
      throw appError;
    }

    log(
      'loaded backend snapshot achievementCount=%d',
      Array.isArray((response.data as { achievements?: unknown[] })?.achievements)
        ? (response.data as { achievements: unknown[] }).achievements.length
        : 0
    );
    return normalizeRewardsSnapshot(response.data);
  },

  async disconnectDiscord(): Promise<void> {
    let response: ApiResponse<unknown>;
    try {
      response = await apiClient.delete<ApiResponse<unknown>>('/rewards/discord', {
        timeout: REWARDS_SNAPSHOT_TIMEOUT_MS,
      });
    } catch (transportError) {
      const normalized = normalizeRewardsApiError(transportError);
      log('disconnect transport failed error=%s', normalized.error);
      throw normalized;
    }

    if (!response.success) {
      const appError: RewardsApiError = {
        success: false,
        error: response.error ?? response.message ?? 'Unable to disconnect Discord',
      };
      log('disconnect backend error error=%s', appError.error);
      throw appError;
    }

    log('discord disconnected');
  },
};
