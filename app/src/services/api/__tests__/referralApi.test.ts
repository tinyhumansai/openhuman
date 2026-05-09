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

  it('claimReferral calls core with trimmed code and fingerprint', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockResolvedValueOnce({});
    await referralApi.claimReferral('  abcd  ');
    expect(callCoreCommand).toHaveBeenCalledWith(
      'openhuman.referral_claim',
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

  it('claimReferral throws { success: false, error } preserving err.error string', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce({ error: 'Code already used' });
    await expect(referralApi.claimReferral('ABCD')).rejects.toEqual({
      success: false,
      error: 'Code already used',
    });
  });
});

describe('normalizeReferralStats error conditions', () => {
  it('handles invalid money types gracefully', () => {
    const stats = normalizeReferralStats({
      totals: { totalRewardUsd: { someOtherField: '1.23' } },
      referrals: [{ status: 'converted', rewardUsd: { noNumberDecimal: true } }],
    });
    expect(stats.totals.totalRewardUsd).toBe(0);
    expect(stats.referrals[0].rewardUsd).toBeUndefined();
  });
});

describe('referralApi edge cases', () => {
  it('claimReferral throws when code is empty', async () => {
    await expect(referralApi.claimReferral('   ')).rejects.toEqual({
      success: false,
      error: 'Referral code is required',
    });
  });
});

describe('referralRpcErrorMessage edge cases', () => {
  it('handles object with only message', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce({ message: 'A message error' });
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'A message error',
    });
  });

  it('handles string errors', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce('Some primitive string error');
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'Some primitive string error',
    });
  });

  it('handles empty Error object gracefully', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    const emptyErr = new Error();
    emptyErr.message = ''; // explicitly make message empty
    vi.mocked(callCoreCommand).mockRejectedValueOnce(emptyErr);
    await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: 'Error' });
  });
});

it('handles Error object with message', async () => {
  const { callCoreCommand } = await import('../../coreCommandClient');
  vi.mocked(callCoreCommand).mockRejectedValueOnce(new Error('A custom error message'));
  await expect(referralApi.getStats()).rejects.toEqual({
    success: false,
    error: 'A custom error message',
  });
});

it('handles custom class errors with message', async () => {
  class CustomError {
    message = 'Custom error class message';
  }
  const { callCoreCommand } = await import('../../coreCommandClient');
  vi.mocked(callCoreCommand).mockRejectedValueOnce(new CustomError());
  await expect(referralApi.getStats()).rejects.toEqual({
    success: false,
    error: 'Custom error class message',
  });
});

it('handles explicit Error object to cover branch', async () => {
  const { callCoreCommand } = await import('../../coreCommandClient');
  // We explicitly throw an Error without any properties that could trigger earlier returns
  // Actually getStats test earlier throws new Error('Core RPC HTTP 503') which covers this.
  // The missing coverage on line 24 is due to err.message being checked.
  const err = new Error('Direct error message test');
  vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
  await expect(referralApi.getStats()).rejects.toEqual({
    success: false,
    error: 'Direct error message test',
  });
});

it('handles Error object with empty message gracefully', async () => {
  const { callCoreCommand } = await import('../../coreCommandClient');
  // We explicitly throw an Error without any properties that could trigger earlier returns
  const err = new Error('');
  vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
  await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: 'Error' });
});

it('handles empty Error object message', async () => {
  const { callCoreCommand } = await import('../../coreCommandClient');
  const err = new Error('');
  vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
  await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: 'Error' });
});

describe('referralRpcErrorMessage missing branch', () => {
  it('covers the exact Error message truthy branch directly', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    // Ensure err is NOT an object with 'error' or 'message' string properties that match first
    // We can do this by throwing an Error object but overriding its properties or just using a plain Error.
    const e = new Error('direct hit');
    Object.defineProperty(e, 'message', { value: 'direct hit', enumerable: false });
    vi.mocked(callCoreCommand).mockRejectedValueOnce(e);
    await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: 'direct hit' });
  });
});

it('covers the exact err.message branch for Error', async () => {
  const { callCoreCommand } = await import('../../coreCommandClient');
  // An error where typeof err !== 'object' but err instanceof Error ? Impossible in JS since typeof Error is object.
  // Ah, wait: `typeof err === 'object'` is true.
  // Then it checks `const o = err as Record<string, unknown>;`
  // Then it checks `typeof o.error === 'string'` and `typeof o.message === 'string'`.
  // Wait, if it has `err.message` which is a string, it will return `o.message` on line 18!
  // So line 24 is UNREACHABLE if `err.message` is a string!
  // Because `err` is an object, and `err.message` is a string, line 17: `if (typeof o.message === 'string' && o.message.trim() !== '') return o.message;` handles it.
  // Unless `err.message` is empty string after trim(), but truthy? `err.message` is a string, if it's truthy, it's not empty string.
  // Wait, what if `err.message` is '   ' (spaces)?
  // `o.message.trim() !== ''` is false.
  // Then it goes to line 23: `if (err instanceof Error && err.message)`.
  // `err.message` is truthy ('   '). So it enters the block and returns `err.message` ('   ')!
  // Let's test this exact scenario to hit line 24!
  const e = new Error('   ');
  vi.mocked(callCoreCommand).mockRejectedValueOnce(e);
  await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: '   ' });
});

describe('normalizeReferralStats error and edge conditions', () => {
  it('handles invalid money types gracefully, defaulting to 0', () => {
    const stats = normalizeReferralStats({
      totals: { totalRewardUsd: { unexpectedStructure: '1.23' } },
      referrals: [{ status: 'converted', rewardUsd: { noNumberDecimal: true } }],
    });
    expect(stats.totals.totalRewardUsd).toBe(0);
    expect(stats.referrals[0].rewardUsd).toBeUndefined();
  });

  it('maps applied_referral_code snake_case properly', () => {
    const stats = normalizeReferralStats({
      applied_referral_code: 'USEDCODE',
      can_apply_referral: false,
    });
    expect(stats.appliedReferralCode).toBe('USEDCODE');
    expect(stats.canApplyReferral).toBe(false);
  });
});

describe('referralApi edge cases', () => {
  it('claimReferral throws when code is empty string', async () => {
    await expect(referralApi.claimReferral('   ')).rejects.toEqual({
      success: false,
      error: 'Referral code is required',
    });
  });
});

describe('referralRpcErrorMessage edge cases', () => {
  it('handles object with message but no error property', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce({ message: 'A structured message error' });
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'A structured message error',
    });
  });

  it('handles primitive string errors', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce('Some primitive string error');
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'Some primitive string error',
    });
  });

  it('handles explicit Error object with missing message', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    const err = new Error('');
    vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
    await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: 'Error' });
  });

  it('handles Error object with whitespace message gracefully', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    const err = new Error('   ');
    vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
    await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: '   ' });
  });
});

describe('normalizeReferralStats error and edge conditions', () => {
  it('handles invalid money types gracefully, defaulting to 0', () => {
    const stats = normalizeReferralStats({
      totals: { totalRewardUsd: { unexpectedStructure: '1.23' } },
      referrals: [{ status: 'converted', rewardUsd: { noNumberDecimal: true } }],
    });
    expect(stats.totals.totalRewardUsd).toBe(0);
    expect(stats.referrals[0].rewardUsd).toBeUndefined();
  });

  it('maps applied_referral_code snake_case properly', () => {
    const stats = normalizeReferralStats({
      applied_referral_code: 'USEDCODE',
      can_apply_referral: false,
    });
    expect(stats.appliedReferralCode).toBe('USEDCODE');
    expect(stats.canApplyReferral).toBe(false);
  });
});

describe('referralApi edge cases', () => {
  it('claimReferral throws when code is empty string', async () => {
    await expect(referralApi.claimReferral('   ')).rejects.toEqual({
      success: false,
      error: 'Referral code is required',
    });
  });
});

describe('referralRpcErrorMessage edge cases', () => {
  it('handles object with message but no error property', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce({ message: 'A structured message error' });
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'A structured message error',
    });
  });

  it('handles primitive string errors', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    vi.mocked(callCoreCommand).mockRejectedValueOnce('Some primitive string error');
    await expect(referralApi.getStats()).rejects.toEqual({
      success: false,
      error: 'Some primitive string error',
    });
  });

  it('handles explicit Error object with missing message', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    const err = new Error('');
    vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
    await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: 'Error' });
  });

  it('handles Error object with whitespace message gracefully', async () => {
    const { callCoreCommand } = await import('../../coreCommandClient');
    const err = new Error('   ');
    vi.mocked(callCoreCommand).mockRejectedValueOnce(err);
    await expect(referralApi.getStats()).rejects.toEqual({ success: false, error: '   ' });
  });
});
