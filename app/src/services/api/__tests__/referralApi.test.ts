import { describe, expect, it, vi } from 'vitest';

import { normalizeReferralStats, referralApi } from '../referralApi';

vi.mock('../../coreCommandClient', () => ({ callCoreCommand: vi.fn() }));

describe('normalizeReferralStats', () => {
  it('maps camelCase stats and referral rows', () => {
    const stats = normalizeReferralStats({
      referralCode: 'ABC12',
      referralLink: 'https://app.example/r/ABC12',
      totals: { totalRewardUsd: 12.5, pendingCount: 1, convertedCount: 2 },
      referrals: [
        { referredUserId: 'u1', status: 'pending', createdAt: '2025-01-01' },
        { referredUserId: 'u2', status: 'converted', convertedAt: '2025-01-02', rewardUsd: 5 },
      ],
      appliedReferralCode: null,
      canApplyReferral: true,
      rewardRateBps: 2000,
    });
    expect(stats.referralCode).toBe('ABC12');
    expect(stats.referralLink).toBe('https://app.example/r/ABC12');
    expect(stats.totals).toEqual({ totalRewardUsd: 12.5, pendingCount: 1, convertedCount: 2 });
    expect(stats.referrals).toHaveLength(2);
    expect(stats.referrals[0].status).toBe('pending');
    expect(stats.referrals[1].status).toBe('converted');
    expect(stats.referrals[1].rewardUsd).toBe(5);
    expect(stats.appliedReferralCode).toBeNull();
    expect(stats.canApplyReferral).toBe(true);
    expect(stats.rewardRateBps).toBe(2000);
  });

  it('maps snake_case and coerces unknown status to pending', () => {
    const stats = normalizeReferralStats({
      code: 'X',
      link: 'https://x',
      summary: { total_reward_usd: '3.25', pending_referrals: 2, converted_referrals: 0 },
      referralRows: [{ status: 'weird', _id: 'r1' }],
    });
    expect(stats.referralCode).toBe('X');
    expect(stats.totals.totalRewardUsd).toBe(3.25);
    expect(stats.totals.pendingCount).toBe(2);
    expect(stats.referrals[0].status).toBe('pending');
    expect(stats.referrals[0].id).toBe('r1');
  });

  it('handles empty payload', () => {
    const stats = normalizeReferralStats({});
    expect(stats.referralCode).toBe('');
    expect(stats.referrals).toEqual([]);
    expect(stats.totals.totalRewardUsd).toBe(0);
  });

  it('maps completed status to converted and rewardAmountUsd', () => {
    const stats = normalizeReferralStats({
      referrals: [{ status: 'Completed', rewardAmountUsd: 2.5, referredUserId: 'u1' }],
      totals: { totalRewardUsd: 0, pendingCount: 0, convertedCount: 0 },
    });
    expect(stats.referrals[0].status).toBe('converted');
    expect(stats.referrals[0].rewardUsd).toBe(2.5);
    expect(stats.totals.convertedCount).toBe(1);
    expect(stats.totals.totalRewardUsd).toBe(2.5);
  });

  it('maps referralId, joinedAt, referredUserMasked, and Joined status', () => {
    const stats = normalizeReferralStats({
      referrals: [
        {
          referralId: 'ref-99',
          status: 'Joined',
          referredUserMasked: '  j***@gmail.com  ',
          joinedAt: '2026-04-01T12:00:00.000Z',
          convertedAt: null,
        },
      ],
    });
    expect(stats.referrals[0].id).toBe('ref-99');
    expect(stats.referrals[0].referredUserMasked).toBe('j***@gmail.com');
    expect(stats.referrals[0].status).toBe('pending');
    expect(stats.referrals[0].createdAt).toBe('2026-04-01T12:00:00.000Z');
  });

  it('maps referred_user_masked snake_case', () => {
    const stats = normalizeReferralStats({
      referrals: [{ referred_user_masked: 'U***', status: 'Converted' }],
    });
    expect(stats.referrals[0].referredUserMasked).toBe('U***');
    expect(stats.referrals[0].status).toBe('converted');
  });

  it('reads Mongo-style Decimal128 and nested transactions', () => {
    const stats = normalizeReferralStats({
      referrals: [
        {
          status: 'converted',
          referred_user_id: { $oid: '507f1f77bcf86cd799439011' },
          transactions: [
            { rewardAmountUsd: { $numberDecimal: '1.25' } },
            { reward_amount_usd: '0.75' },
          ],
        },
      ],
    });
    expect(stats.referrals[0].referredUserId).toBe('507f1f77bcf86cd799439011');
    expect(stats.referrals[0].rewardUsd).toBe(2);
    expect(stats.totals.totalRewardUsd).toBe(2);
    expect(stats.totals.convertedCount).toBe(1);
  });

  it('prefers explicit totals when backend sends them', () => {
    const stats = normalizeReferralStats({
      totals: { totalRewardUsd: 10, pendingCount: 0, convertedCount: 2 },
      referrals: [{ status: 'converted', rewardUsd: 3 }],
    });
    expect(stats.totals.totalRewardUsd).toBe(10);
    expect(stats.totals.convertedCount).toBe(2);
  });

  it('maps totalRewardsEarnedUsd from backend stats payload', () => {
    const stats = normalizeReferralStats({
      totals: { totalRewardsEarnedUsd: 4.5, pendingCount: 0, convertedCount: 1 },
      referrals: [{ status: 'converted' }],
    });
    expect(stats.totals.totalRewardUsd).toBe(4.5);
  });
});

describe('referralApi', () => {
  it('getStats normalizes core RPC payload', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockResolvedValueOnce({
      referralCode: 'Z9',
      referralLink: 'https://z',
      totals: { totalRewardUsd: 1, pendingCount: 0, convertedCount: 1 },
      referrals: [],
    });
    const out = await referralApi.getStats();
    expect(callCoreCommand).toHaveBeenCalledWith('openhuman.referral_get_stats');
    expect(out.referralCode).toBe('Z9');
  });

  it('applyCode calls core with trimmed code and fingerprint', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockResolvedValueOnce({});
    await referralApi.applyCode('  abcd  ');
    expect(callCoreCommand).toHaveBeenCalledWith(
      'openhuman.referral_apply',
      expect.objectContaining({ code: 'abcd', deviceFingerprint: expect.any(String) })
    );
  });

  it('getStats throws { success: false, error } when core rejects with Error', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce(new Error('Core RPC HTTP 503'));
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'Core RPC HTTP 503',
    });
  });

  it('applyCode throws { success: false, error } preserving err.error string', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce({ error: 'Code already used' });
    await expect(referralApi.applyCode('ABCD')).rejects.toEqual({
      success: false,
      error: 'Code already used',
    });
  });
});
