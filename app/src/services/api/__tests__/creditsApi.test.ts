import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreCommand = vi.fn();

vi.mock('../../coreCommandClient', () => ({
  callCoreCommand: (...args: unknown[]) => mockCallCoreCommand(...args),
}));

const { creditsApi, normalizeCouponRedeemResult, normalizeRedeemedCoupon, normalizeTeamUsage } =
  await import('../creditsApi');

describe('creditsApi coupon helpers', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('normalizes redeem payloads from backend data shape', () => {
    expect(
      normalizeCouponRedeemResult({ couponCode: 'SAVE-2026', amount_usd: '4.5', pending: 0 })
    ).toEqual({ couponCode: 'SAVE-2026', amountUsd: 4.5, pending: false });
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
  it('passes through well-formed camelCase fields', () => {
    const input = {
      remainingUsd: 12.5,
      cycleBudgetUsd: 25,
      cycleLimit5hr: 3.2,
      cycleLimit7day: 18,
      fiveHourCapUsd: 5,
      fiveHourResetsAt: '2026-04-09T18:00:00Z',
      cycleStartDate: '2026-04-07T00:00:00Z',
      cycleEndsAt: '2026-04-14T00:00:00Z',
      bypassCycleLimit: false,
    };
    expect(normalizeTeamUsage(input)).toEqual(input);
  });

  it('maps snake_case backend fields to camelCase', () => {
    const result = normalizeTeamUsage({
      remaining_usd: 10,
      cycle_budget_usd: 20,
      five_hour_spend_usd: 2.5,
      cycle_limit_7day: 15,
      five_hour_cap_usd: 5,
      five_hour_resets_at: '2026-04-09T18:00:00Z',
      cycle_start_date: '2026-04-07T00:00:00Z',
      cycle_ends_at: '2026-04-14T00:00:00Z',
      bypass_cycle_limit: true,
    });
    expect(result.remainingUsd).toBe(10);
    expect(result.cycleBudgetUsd).toBe(20);
    expect(result.cycleLimit5hr).toBe(2.5);
    expect(result.cycleLimit7day).toBe(15);
    expect(result.fiveHourCapUsd).toBe(5);
    expect(result.fiveHourResetsAt).toBe('2026-04-09T18:00:00Z');
    expect(result.bypassCycleLimit).toBe(true);
  });

  it('maps legacy fiveHourSpendUsd to cycleLimit5hr', () => {
    const result = normalizeTeamUsage({ fiveHourSpendUsd: 4.0 });
    expect(result.cycleLimit5hr).toBe(4.0);
  });

  it('returns safe defaults for empty object', () => {
    const result = normalizeTeamUsage({});
    expect(result.remainingUsd).toBe(0);
    expect(result.cycleBudgetUsd).toBe(0);
    expect(result.cycleLimit5hr).toBe(0);
    expect(result.cycleLimit7day).toBe(0);
    expect(result.fiveHourCapUsd).toBe(0);
    expect(result.fiveHourResetsAt).toBeNull();
    expect(result.bypassCycleLimit).toBe(false);
    expect(typeof result.cycleStartDate).toBe('string');
    expect(typeof result.cycleEndsAt).toBe('string');
  });

  it('does not crash on null or undefined input', () => {
    expect(() => normalizeTeamUsage(null)).not.toThrow();
    expect(() => normalizeTeamUsage(undefined)).not.toThrow();
    const result = normalizeTeamUsage(null);
    expect(result.remainingUsd).toBe(0);
    expect(result.cycleLimit5hr).toBe(0);
  });

  it('getTeamUsage normalizes the RPC response', async () => {
    mockCallCoreCommand.mockResolvedValueOnce({ remaining_usd: 8, cycle_budget_usd: 25 });

    const result = await creditsApi.getTeamUsage();
    expect(result.remainingUsd).toBe(8);
    expect(result.cycleBudgetUsd).toBe(25);
    expect(result.cycleLimit5hr).toBe(0);
    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.team_get_usage');
  });
});
