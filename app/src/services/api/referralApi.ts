import type {
  ReferralRelationshipStatus,
  ReferralRow,
  ReferralStats,
  ReferralStatsTotals,
} from '../../types/referral';
import { getOrCreateDeviceFingerprint } from '../../utils/deviceFingerprint';
import { callCoreCommand } from '../coreCommandClient';

/** Shape thrown by {@link referralApi.getStats} / {@link referralApi.applyCode} on RPC failure. */
export type ReferralRpcFailure = { success: false; error: string };

function referralRpcErrorMessage(err: unknown): string {
  if (err && typeof err === 'object') {
    const o = err as Record<string, unknown>;
    if (typeof o.error === 'string' && o.error.trim() !== '') {
      return o.error;
    }
    if (typeof o.message === 'string' && o.message.trim() !== '') {
      return o.message;
    }
  }
  if (err instanceof Error && err.message) {
    return err.message;
  }
  return String(err);
}

function throwReferralRpcFailure(err: unknown): never {
  const failure: ReferralRpcFailure = { success: false, error: referralRpcErrorMessage(err) };
  throw failure;
}

function num(v: unknown): number {
  if (typeof v === 'number' && Number.isFinite(v)) return v;
  if (typeof v === 'string' && v.trim() !== '') {
    const n = Number(v);
    return Number.isFinite(n) ? n : 0;
  }
  return 0;
}

/** Mongo Decimal128 in JSON (`{ $numberDecimal: "1.23" }`) and similar. */
function coerceMoney(v: unknown): number {
  if (v === null || v === undefined) return 0;
  if (typeof v === 'number' && Number.isFinite(v)) return v;
  if (typeof v === 'string' && v.trim() !== '') {
    const n = Number(v);
    return Number.isFinite(n) ? n : 0;
  }
  const o = asRecord(v);
  if (o && typeof o.$numberDecimal === 'string') {
    return num(o.$numberDecimal);
  }
  return 0;
}

function coerceId(v: unknown): string | undefined {
  if (typeof v === 'string' && v.trim()) return v.trim();
  const o = asRecord(v);
  if (o && typeof o.$oid === 'string' && o.$oid.trim()) return o.$oid.trim();
  return undefined;
}

function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function normalizeStatus(raw: unknown): ReferralRelationshipStatus {
  const s = typeof raw === 'string' ? raw.toLowerCase().trim() : '';
  if (s === 'converted' || s === 'completed' || s === 'complete') return 'converted';
  if (s === 'joined') return 'pending';
  if (s === 'pending' || s === 'expired') return s;
  return 'pending';
}

function rowRewardUsd(r: Record<string, unknown>): number {
  const direct = coerceMoney(
    r.rewardUsd ??
      r.reward_usd ??
      r.rewardAmountUsd ??
      r.reward_amount_usd ??
      r.totalRewardUsd ??
      r.total_reward_usd
  );
  const cents = num(r.rewardCents ?? r.reward_cents);
  const fromCents = cents > 0 ? cents / 100 : 0;

  const txs = r.transactions ?? r.referralTransactions ?? r.referral_transactions;
  let txSum = 0;
  if (Array.isArray(txs)) {
    for (const t of txs) {
      const tr = asRecord(t) ?? {};
      const m = coerceMoney(tr.rewardAmountUsd ?? tr.reward_amount_usd ?? tr.rewardUsd);
      const c = num(tr.rewardCents ?? tr.reward_cents);
      txSum += m > 0 ? m : c > 0 ? c / 100 : 0;
    }
  }

  if (txSum > 0) return txSum;
  if (direct > 0) return direct;
  return fromCents;
}

function normalizeRow(entry: unknown): ReferralRow {
  const r = asRecord(entry) ?? {};
  const refUser = asRecord(r.referredUser ?? r.referred_user ?? r.user);
  const rewardUsd = rowRewardUsd(r);
  const referredUserId =
    (typeof r.referredUserId === 'string' && r.referredUserId) ||
    (typeof r.referred_user_id === 'string' && r.referred_user_id) ||
    (typeof r.refereeId === 'string' && r.refereeId) ||
    (typeof r.referee_id === 'string' && r.referee_id) ||
    coerceId(r.referredUserId) ||
    coerceId(r.referred_user_id) ||
    (refUser
      ? (coerceId(refUser._id) ?? (typeof refUser.id === 'string' ? refUser.id : undefined))
      : undefined);
  const referredDisplayName =
    typeof r.referredDisplayName === 'string'
      ? r.referredDisplayName
      : typeof r.referred_display_name === 'string'
        ? r.referred_display_name
        : typeof r.referredUsername === 'string'
          ? r.referredUsername
          : refUser && typeof refUser.username === 'string'
            ? refUser.username
            : undefined;

  const referredUserMaskedRaw =
    typeof r.referredUserMasked === 'string'
      ? r.referredUserMasked
      : typeof r.referred_user_masked === 'string'
        ? r.referred_user_masked
        : undefined;
  const referredUserMasked =
    referredUserMaskedRaw && referredUserMaskedRaw.trim() !== ''
      ? referredUserMaskedRaw.trim()
      : undefined;

  return {
    id:
      (typeof r.referralId === 'string' && r.referralId) ||
      (typeof r.referral_id === 'string' && r.referral_id) ||
      (typeof r.id === 'string' && r.id) ||
      (typeof r._id === 'string' && r._id) ||
      coerceId(r._id),
    referredUserId,
    status: normalizeStatus(r.status),
    referralCode:
      typeof r.referralCode === 'string'
        ? r.referralCode
        : typeof r.referral_code === 'string'
          ? r.referral_code
          : undefined,
    createdAt:
      typeof r.joinedAt === 'string'
        ? r.joinedAt
        : typeof r.joined_at === 'string'
          ? r.joined_at
          : typeof r.createdAt === 'string'
            ? r.createdAt
            : typeof r.created_at === 'string'
              ? r.created_at
              : undefined,
    convertedAt:
      r.convertedAt === null
        ? null
        : typeof r.convertedAt === 'string'
          ? r.convertedAt
          : typeof r.converted_at === 'string'
            ? r.converted_at
            : undefined,
    rewardUsd: rewardUsd > 0 ? rewardUsd : undefined,
    referredDisplayName,
    referredUserMasked,
  };
}

function deriveTotalsFromReferrals(referrals: ReferralRow[]): ReferralStatsTotals {
  let totalRewardUsd = 0;
  let pendingCount = 0;
  let convertedCount = 0;
  for (const row of referrals) {
    if (row.rewardUsd != null && row.rewardUsd > 0) {
      totalRewardUsd += row.rewardUsd;
    }
    if (row.status === 'pending') pendingCount += 1;
    if (row.status === 'converted') convertedCount += 1;
  }
  return { totalRewardUsd, pendingCount, convertedCount };
}

/**
 * Map backend `/referral/stats` payload (flexible field names) to UI types.
 */
export function normalizeReferralStats(raw: unknown): ReferralStats {
  const r = asRecord(raw) ?? {};
  const code = String(r.referralCode ?? r.code ?? '').trim();
  const link = String(r.referralLink ?? r.link ?? '').trim();

  const totalsRaw = asRecord(r.totals) ?? asRecord(r.summary) ?? {};
  const totalFromApi = Math.max(
    coerceMoney(
      totalsRaw.totalRewardsEarnedUsd ??
        totalsRaw.total_rewards_earned_usd ??
        totalsRaw.totalRewardUsd ??
        totalsRaw.total_reward_usd ??
        totalsRaw.lifetimeRewardUsd ??
        totalsRaw.lifetime_reward_usd
    ),
    coerceMoney(
      r.totalRewardsEarnedUsd ??
        r.total_rewards_earned_usd ??
        r.totalRewardUsd ??
        r.total_reward_usd ??
        r.totalEarningsUsd ??
        r.total_earnings_usd ??
        r.earningsUsd ??
        r.earnings_usd ??
        r.rewardsTotalUsd ??
        r.rewards_total_usd
    )
  );

  const pendingFromApi = Math.round(
    num(
      totalsRaw.pendingCount ??
        totalsRaw.pending_count ??
        totalsRaw.pendingReferrals ??
        totalsRaw.pending_referrals ??
        r.pendingCount ??
        r.pending_count
    )
  );
  const convertedFromApi = Math.round(
    num(
      totalsRaw.convertedCount ??
        totalsRaw.converted_count ??
        totalsRaw.convertedReferrals ??
        totalsRaw.converted_referrals ??
        r.convertedCount ??
        r.converted_count
    )
  );

  const listRaw = r.referrals ?? r.referralRows ?? r.rows;
  const referrals: ReferralRow[] = Array.isArray(listRaw) ? listRaw.map(normalizeRow) : [];

  const derived = deriveTotalsFromReferrals(referrals);
  const totals: ReferralStatsTotals = {
    totalRewardUsd: totalFromApi > 0 ? totalFromApi : derived.totalRewardUsd,
    pendingCount: pendingFromApi || derived.pendingCount,
    convertedCount: convertedFromApi || derived.convertedCount,
  };

  const appliedReferralCode =
    r.appliedReferralCode === null
      ? null
      : typeof r.appliedReferralCode === 'string'
        ? r.appliedReferralCode
        : typeof r.applied_referral_code === 'string'
          ? r.applied_referral_code
          : undefined;

  const canApplyReferral =
    typeof r.canApplyReferral === 'boolean'
      ? r.canApplyReferral
      : typeof r.can_apply_referral === 'boolean'
        ? r.can_apply_referral
        : undefined;

  const rewardRateBps = num(r.rewardRateBps ?? r.reward_rate_bps);

  return {
    referralCode: code,
    referralLink: link,
    totals,
    referrals,
    appliedReferralCode,
    canApplyReferral,
    rewardRateBps: rewardRateBps > 0 ? rewardRateBps : undefined,
  };
}

export const referralApi = {
  /**
   * Referral stats via core RPC (`openhuman.referral_get_stats` → backend GET /referral/stats).
   * Uses the sidecar HTTP client so the desktop WebView avoids direct `fetch` (fixes WKWebView "Load failed" / CORS to the API host).
   */
  getStats: async (): Promise<ReferralStats> => {
    try {
      const data = await callCoreCommand<unknown>('openhuman.referral_get_stats');
      console.debug('[referral] stats loaded via core', {
        hasCode: !!(data && typeof data === 'object'),
      });
      return normalizeReferralStats(data);
    } catch (err) {
      console.debug('[referral] getStats RPC failed', referralRpcErrorMessage(err));
      throwReferralRpcFailure(err);
    }
  },

  /**
   * Apply referral code via core RPC (`openhuman.referral_apply` → backend POST /referral/apply).
   */
  applyCode: async (code: string): Promise<void> => {
    const trimmed = code.trim();
    if (!trimmed) {
      throw { success: false as const, error: 'Referral code is required' };
    }
    const deviceFingerprint = getOrCreateDeviceFingerprint();
    try {
      await callCoreCommand<unknown>('openhuman.referral_apply', {
        code: trimmed,
        deviceFingerprint,
      });
      console.debug('[referral] apply succeeded', { codeLength: trimmed.length });
    } catch (err) {
      console.debug('[referral] apply RPC failed', referralRpcErrorMessage(err));
      throwReferralRpcFailure(err);
    }
  },
};
