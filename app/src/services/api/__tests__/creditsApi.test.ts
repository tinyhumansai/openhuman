import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreCommand = vi.fn();

vi.mock('../../coreCommandClient', () => ({
  callCoreCommand: (...args: unknown[]) => mockCallCoreCommand(...args),
}));

const { creditsApi, normalizeCouponRedeemResult, normalizeRedeemedCoupon, normalizeTeamUsage } =
  await import('../creditsApi');

describe('normalizeCouponRedeemResult', () => {
  it('normalizes redeem payloads from backend data shape', () => {
    expect(
      normalizeCouponRedeemResult({ couponCode: 'SAVE-2026', amount_usd: '4.5', pending: 0 })
    ).toEqual({ couponCode: 'SAVE-2026', amountUsd: 4.5, pending: false });
  });

  it('returns safe defaults for empty object', () => {
    expect(normalizeCouponRedeemResult({})).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: false,
    });
  });

  it('returns safe defaults for null/undefined/non-object inputs', () => {
    const expected = { couponCode: '', amountUsd: 0, pending: false };
    expect(normalizeCouponRedeemResult(null)).toEqual(expected);
    expect(normalizeCouponRedeemResult(undefined)).toEqual(expected);
    expect(normalizeCouponRedeemResult('not an object')).toEqual(expected);
    expect(normalizeCouponRedeemResult(123)).toEqual(expected);
    expect(normalizeCouponRedeemResult([])).toEqual(expected);
  });

  it('unwraps nested { data: ... } envelopes', () => {
    expect(
      normalizeCouponRedeemResult({
        data: { couponCode: 'NESTED-20', amountUsd: 20, pending: true },
      })
    ).toEqual({ couponCode: 'NESTED-20', amountUsd: 20, pending: true });
  });

  it('ignores data field if it is not an object', () => {
    expect(
      normalizeCouponRedeemResult({ data: 'not an object', couponCode: 'TOP-LEVEL', amountUsd: 15 })
    ).toEqual({ couponCode: 'TOP-LEVEL', amountUsd: 15, pending: false });
  });

  it('falls back to code if couponCode is missing or invalid', () => {
    expect(normalizeCouponRedeemResult({ code: 'CODE-ONLY', amountUsd: 10 })).toEqual({
      couponCode: 'CODE-ONLY',
      amountUsd: 10,
      pending: false,
    });

    // couponCode exists but is empty/whitespace, should fall back to code
    expect(
      normalizeCouponRedeemResult({ couponCode: '   ', code: 'FALLBACK', amountUsd: 10 })
    ).toEqual({ couponCode: 'FALLBACK', amountUsd: 10, pending: false });

    // couponCode exists but is not a string
    expect(
      normalizeCouponRedeemResult({ couponCode: 123, code: 'FALLBACK2', amountUsd: 10 })
    ).toEqual({ couponCode: 'FALLBACK2', amountUsd: 10, pending: false });
  });

  it('trims whitespace from coupon codes', () => {
    expect(normalizeCouponRedeemResult({ couponCode: '  SPACEY  ', amountUsd: 5 })).toEqual({
      couponCode: 'SPACEY',
      amountUsd: 5,
      pending: false,
    });

    expect(normalizeCouponRedeemResult({ code: '  SPACEY-CODE  ', amountUsd: 5 })).toEqual({
      couponCode: 'SPACEY-CODE',
      amountUsd: 5,
      pending: false,
    });
  });

  it('normalizes amountUsd vs amount_usd', () => {
    expect(normalizeCouponRedeemResult({ amountUsd: 5.5 })).toEqual({
      couponCode: '',
      amountUsd: 5.5,
      pending: false,
    });

    expect(normalizeCouponRedeemResult({ amount_usd: 6.5 })).toEqual({
      couponCode: '',
      amountUsd: 6.5,
      pending: false,
    });

    // Valid string amounts
    expect(normalizeCouponRedeemResult({ amountUsd: '7.5' })).toEqual({
      couponCode: '',
      amountUsd: 7.5,
      pending: false,
    });

    // amountUsd should take precedence over amount_usd if both exist
    expect(normalizeCouponRedeemResult({ amountUsd: 8.5, amount_usd: 9.5 })).toEqual({
      couponCode: '',
      amountUsd: 8.5,
      pending: false,
    });
  });

  it('handles truthy/falsy values for pending', () => {
    expect(normalizeCouponRedeemResult({ pending: true })).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: true,
    });

    expect(normalizeCouponRedeemResult({ pending: 1 })).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: true,
    });

    expect(normalizeCouponRedeemResult({ pending: 'yes' })).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: true,
    });

    expect(normalizeCouponRedeemResult({ pending: false })).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: false,
    });

    expect(normalizeCouponRedeemResult({ pending: 0 })).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: false,
    });

    expect(normalizeCouponRedeemResult({ pending: null })).toEqual({
      couponCode: '',
      amountUsd: 0,
      pending: false,
    });
  });
});

describe('creditsApi coupon helpers', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('normalizes redeemed coupon rows', () => {
    expect(
      normalizeRedeemedCoupon({
        code: 'HELLO123',
        amountUsd: '7.25',
        redeemed_at: '2026-04-09T12:00:00.000Z',
        activation_type: 'CONDITIONAL',
        activation_condition: 'SUBSCRIBE_PAID_PLAN',
        fulfilled: false,
      })
    ).toEqual({
      code: 'HELLO123',
      amountUsd: 7.25,
      redeemedAt: '2026-04-09T12:00:00.000Z',
      activationType: 'CONDITIONAL',
      activationCondition: 'SUBSCRIBE_PAID_PLAN',
      fulfilled: false,
      fulfilledAt: null,
    });
  });

  it('normalizes redeemed coupon handles null/undefined/empty object safely', () => {
    const expectedDefaults = {
      code: '',
      amountUsd: 0,
      redeemedAt: null,
      activationType: 'IMMEDIATE',
      fulfilled: false,
      fulfilledAt: null,
      activationCondition: null,
    };
    expect(normalizeRedeemedCoupon(null)).toEqual(expectedDefaults);
    expect(normalizeRedeemedCoupon(undefined)).toEqual(expectedDefaults);
    expect(normalizeRedeemedCoupon({})).toEqual(expectedDefaults);
  });

  it('normalizes redeemed coupon falls back to couponCode if code is absent or empty', () => {
    expect(normalizeRedeemedCoupon({ couponCode: 'FALLBACK-CODE' }).code).toBe('FALLBACK-CODE');
    expect(normalizeRedeemedCoupon({ code: '  ', couponCode: 'FALLBACK-CODE' }).code).toBe(
      'FALLBACK-CODE'
    );
  });

  it('normalizes redeemed coupon handles camelCase variants of fields', () => {
    expect(
      normalizeRedeemedCoupon({
        code: 'CAMEL',
        amountUsd: 10,
        redeemedAt: '2026-04-10T12:00:00.000Z',
        activationType: 'MANUAL',
        fulfilledAt: '2026-04-10T12:05:00.000Z',
        activationCondition: 'SOME_CONDITION',
      })
    ).toEqual({
      code: 'CAMEL',
      amountUsd: 10,
      redeemedAt: '2026-04-10T12:00:00.000Z',
      activationType: 'MANUAL',
      fulfilled: false,
      fulfilledAt: '2026-04-10T12:05:00.000Z',
      activationCondition: 'SOME_CONDITION',
    });
  });

  it('normalizes redeemed coupon coerces fulfilled to boolean', () => {
    expect(normalizeRedeemedCoupon({ fulfilled: 1 }).fulfilled).toBe(true);
    expect(normalizeRedeemedCoupon({ fulfilled: 'yes' }).fulfilled).toBe(true);
    expect(normalizeRedeemedCoupon({ fulfilled: 0 }).fulfilled).toBe(false);
    expect(normalizeRedeemedCoupon({ fulfilled: '' }).fulfilled).toBe(false);
  });

  it('normalizes redeemed coupon handles empty strings as null for nullable string fields', () => {
    const result = normalizeRedeemedCoupon({
      redeemedAt: '   ',
      fulfilledAt: '',
      activationCondition: ' ',
    });
    expect(result.redeemedAt).toBeNull();
    expect(result.fulfilledAt).toBeNull();
    expect(result.activationCondition).toBeNull();
  });

  it('redeemCoupon unwraps and normalizes the core RPC payload', async () => {
    mockCallCoreCommand.mockResolvedValueOnce({
      couponCode: 'APRL-2026',
      amountUsd: 5,
      pending: true,
    });

    await expect(creditsApi.redeemCoupon('APRL-2026')).resolves.toEqual({
      couponCode: 'APRL-2026',
      amountUsd: 5,
      pending: true,
    });

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_redeem_coupon', {
      code: 'APRL-2026',
    });
  });

  it('redeemCoupon also unwraps nested success/data payloads', async () => {
    mockCallCoreCommand.mockResolvedValueOnce({
      success: true,
      data: { code: 'APRL-2026', amountUsd: 5, pending: false },
    });

    await expect(creditsApi.redeemCoupon('APRL-2026')).resolves.toEqual({
      couponCode: 'APRL-2026',
      amountUsd: 5,
      pending: false,
    });

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_redeem_coupon', {
      code: 'APRL-2026',
    });
  });

  it('getUserCoupons normalizes coupon history rows', async () => {
    mockCallCoreCommand.mockResolvedValueOnce([
      {
        code: 'WELCOME',
        amountUsd: 3,
        redeemedAt: '2026-04-09T08:00:00.000Z',
        activationType: 'IMMEDIATE',
        fulfilled: true,
        fulfilledAt: '2026-04-09T08:00:01.000Z',
      },
    ]);

    await expect(creditsApi.getUserCoupons()).resolves.toEqual([
      {
        code: 'WELCOME',
        amountUsd: 3,
        redeemedAt: '2026-04-09T08:00:00.000Z',
        activationType: 'IMMEDIATE',
        activationCondition: null,
        fulfilled: true,
        fulfilledAt: '2026-04-09T08:00:01.000Z',
      },
    ]);

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_get_coupons');
  });
});

describe('normalizeTeamUsage', () => {
  it('passes through well-formed payload', () => {
    const input = {
      remainingUsd: 12.5,
      cycleBudgetUsd: 25,
      cycleSpentUsd: 12.5,
      cycleStartDate: '2026-04-07T00:00:00Z',
      cycleEndsAt: '2026-04-14T00:00:00Z',
      plan: {
        plan: 'BASIC',
        name: 'Basic',
        marginPercent: 25,
        payAsYouGoMarginPercent: 50,
        discountVsPayAsYouGoPercent: 50,
      },
      insights: {
        period: { startDate: '2026-04-07T00:00:00Z', endDate: '2026-04-14T00:00:00Z' },
        totals: {
          inferenceUsd: 8,
          integrationsUsd: 4.5,
          totalUsd: 12.5,
          inferenceCalls: 120,
          integrationCalls: 7,
        },
        dailySeries: [{ date: '2026-04-08', inferenceUsd: 3, integrationsUsd: 1, totalUsd: 4 }],
        topModels: [{ model: 'sonnet-4-6', provider: 'anthropic', spentUsd: 6, calls: 80 }],
        topIntegrations: [{ provider: 'gmail', action: 'send', spentUsd: 2, calls: 3 }],
      },
    };
    expect(normalizeTeamUsage(input)).toEqual(input);
  });

  it('returns safe defaults for empty object', () => {
    const result = normalizeTeamUsage({});
    expect(result.remainingUsd).toBe(0);
    expect(result.cycleBudgetUsd).toBe(0);
    expect(result.cycleSpentUsd).toBe(0);
    expect(result.plan.plan).toBe('FREE');
    expect(result.plan.discountVsPayAsYouGoPercent).toBe(0);
    expect(result.insights.totals.totalUsd).toBe(0);
    expect(result.insights.dailySeries).toEqual([]);
    expect(result.insights.topModels).toEqual([]);
    expect(result.insights.topIntegrations).toEqual([]);
    expect(typeof result.cycleStartDate).toBe('string');
    expect(typeof result.cycleEndsAt).toBe('string');
  });

  it('handles invalid payload types gracefully', () => {
    expect(() => normalizeTeamUsage('string payload')).not.toThrow();
    expect(() => normalizeTeamUsage(12345)).not.toThrow();
    expect(() => normalizeTeamUsage(true)).not.toThrow();
    expect(() => normalizeTeamUsage([])).not.toThrow();
    expect(normalizeTeamUsage('string payload').remainingUsd).toBe(0);
    expect(normalizeTeamUsage(['a', 'b']).remainingUsd).toBe(0);
  });

  it('does not crash on null or undefined input', () => {
    expect(() => normalizeTeamUsage(null)).not.toThrow();
    expect(() => normalizeTeamUsage(undefined)).not.toThrow();
    const result = normalizeTeamUsage(null);
    expect(result.remainingUsd).toBe(0);
    expect(result.insights.totals.totalUsd).toBe(0);
  });

  it('getTeamUsage normalizes the RPC response', async () => {
    mockCallCoreCommand.mockResolvedValueOnce({
      remainingUsd: 8,
      cycleBudgetUsd: 25,
      cycleSpentUsd: 17,
      cycleStartDate: '2026-04-07T00:00:00Z',
      cycleEndsAt: '2026-04-14T00:00:00Z',
    });

    const result = await creditsApi.getTeamUsage();
    expect(result.remainingUsd).toBe(8);
    expect(result.cycleBudgetUsd).toBe(25);
    expect(result.cycleSpentUsd).toBe(17);
    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.team_get_usage');
  });

  it('normalizes insights sub-rows with missing fields to safe defaults', () => {
    const result = normalizeTeamUsage({
      insights: {
        period: { startDate: '2026-05-01', endDate: '2026-05-31' },
        totals: {},
        // Rows with missing optional fields — exercises normalizeDailyPoint,
        // normalizeModelRow, and normalizeIntegrationRow default branches.
        dailySeries: [{ date: '2026-05-01' }],
        topModels: [{ provider: 'anthropic' }],
        topIntegrations: [{ provider: 'gmail' }],
      },
    });

    expect(result.insights.dailySeries).toHaveLength(1);
    expect(result.insights.dailySeries[0].inferenceUsd).toBe(0);
    expect(result.insights.dailySeries[0].integrationsUsd).toBe(0);
    expect(result.insights.dailySeries[0].totalUsd).toBe(0);

    expect(result.insights.topModels).toHaveLength(1);
    expect(result.insights.topModels[0].model).toBe('');
    expect(result.insights.topModels[0].provider).toBe('anthropic');
    expect(result.insights.topModels[0].spentUsd).toBe(0);
    expect(result.insights.topModels[0].calls).toBe(0);

    expect(result.insights.topIntegrations).toHaveLength(1);
    expect(result.insights.topIntegrations[0].action).toBe('');
    expect(result.insights.topIntegrations[0].spentUsd).toBe(0);
    expect(result.insights.topIntegrations[0].calls).toBe(0);
  });

  it('normalizes plan summary with missing fields to safe defaults', () => {
    const result = normalizeTeamUsage({ plan: { name: 'Custom' } });
    expect(result.plan.plan).toBe('FREE');
    expect(result.plan.name).toBe('Custom');
    expect(result.plan.marginPercent).toBe(0);
    expect(result.plan.payAsYouGoMarginPercent).toBe(0);
    expect(result.plan.discountVsPayAsYouGoPercent).toBe(0);
  });

  it('normalizes insights period using cycle dates as fallback when period is absent', () => {
    const result = normalizeTeamUsage({
      cycleStartDate: '2026-05-01T00:00:00Z',
      cycleEndsAt: '2026-05-31T00:00:00Z',
      insights: { totals: {}, dailySeries: [], topModels: [], topIntegrations: [] },
    });
    expect(result.insights.period.startDate).toBe('2026-05-01T00:00:00Z');
    expect(result.insights.period.endDate).toBe('2026-05-31T00:00:00Z');
  });
});
