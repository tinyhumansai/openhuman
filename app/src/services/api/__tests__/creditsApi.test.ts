import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreCommand = vi.fn();

vi.mock('../../coreCommandClient', () => ({
  callCoreCommand: (...args: unknown[]) => mockCallCoreCommand(...args),
}));

const { creditsApi, normalizeCouponRedeemResult, normalizeRedeemedCoupon } =
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
