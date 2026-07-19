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
    username: string | null;
    inviteUrl: string | null;
    membershipStatus: RewardsDiscordMembershipStatus;
  };
  summary: {
    unlockedCount: number;
    totalCount: number;
    assignedDiscordRoleCount: number;
    /** Optional: absent when talking to a backend without the claim feature. */
    claimableCount?: number;
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
  /** Token reward advertised for this achievement (one-time or monthly amount). */
  rewardTokens: number | null;
  /** True when the token reward recurs monthly (subscriber tiers). */
  rewardRecurring: boolean;
  // Claim fields are optional so an older backend (without the claim feature)
  // still yields a valid snapshot; the normalizer always populates them.
  /** True when the user can claim this reward's credit right now. */
  claimable?: boolean;
  /** True when the credit has already been claimed (this calendar month if recurring). */
  claimed?: boolean;
  /** When the reward was claimed (ISO string), or null if not yet claimed. */
  claimedAt?: string | null;
  /** Calendar month (YYYY-MM) a recurring reward's claim state refers to; null for one-time. */
  claimPeriod?: string | null;
}

/** Result of a successful (or idempotent) POST /rewards/claim. */
export interface RewardClaimResult {
  reward: string;
  recurring: boolean;
  period: string | null;
  tokens: number;
  amountUsd: number;
  /** True when the reward had already been claimed (idempotent re-claim). */
  alreadyClaimed: boolean;
  claimedAt: string | null;
  newPromoBalanceUsd: number;
}
