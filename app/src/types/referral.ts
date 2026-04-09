/** Normalized referral relationship status for UI (backend: pending | converted; expired reserved). */
export type ReferralRelationshipStatus = 'pending' | 'converted' | 'expired';

export interface ReferralStatsTotals {
  /** Total USD credited to the referrer from referral rewards */
  totalRewardUsd: number;
  pendingCount: number;
  convertedCount: number;
}

export interface ReferralRow {
  id?: string;
  referredUserId?: string;
  status: ReferralRelationshipStatus;
  referralCode?: string;
  createdAt?: string;
  convertedAt?: string | null;
  /** Reward amount in USD for this relationship when converted */
  rewardUsd?: number;
  /** Optional display name from backend when user id is hidden */
  referredDisplayName?: string;
  /** Masked identity from backend (e.g. j***@gmail.com) — preferred for display */
  referredUserMasked?: string;
}

export interface ReferralStats {
  referralCode: string;
  referralLink: string;
  totals: ReferralStatsTotals;
  referrals: ReferralRow[];
  /** Code this user applied as referred (if any) */
  appliedReferralCode?: string | null;
  /** When false, user likely cannot apply (e.g. already paid); optional from backend */
  canApplyReferral?: boolean;
  rewardRateBps?: number;
}
