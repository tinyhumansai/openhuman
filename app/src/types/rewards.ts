export type RewardsDiscordMembershipStatus =
  | 'member'
  | 'not_in_guild'
  | 'not_linked'
  | 'unavailable';

export type RewardsDiscordRoleStatus =
  | 'assigned'
  | 'not_assigned'
  | 'not_linked'
  | 'not_in_guild'
  | 'not_configured'
  | 'unavailable';

export interface RewardsSnapshot {
  discord: {
    linked: boolean;
    discordId: string | null;
    inviteUrl: string | null;
    membershipStatus: RewardsDiscordMembershipStatus;
  };
  summary: {
    unlockedCount: number;
    totalCount: number;
    assignedDiscordRoleCount: number;
    plan: 'FREE' | 'BASIC' | 'PRO';
    hasActiveSubscription: boolean;
  };
  metrics: {
    currentStreakDays: number;
    longestStreakDays: number;
    cumulativeTokens: number;
    featuresUsedCount: number;
    trackedFeaturesCount: number;
    lastEvaluatedAt: string | null;
    lastSyncedAt: string | null;
  };
  achievements: RewardsAchievement[];
}

export interface RewardsAchievement {
  id: string;
  title: string;
  description: string;
  actionLabel: string;
  unlocked: boolean;
  progressLabel: string;
  roleId: string | null;
  discordRoleStatus: RewardsDiscordRoleStatus;
  creditAmountUsd: number | null;
}
