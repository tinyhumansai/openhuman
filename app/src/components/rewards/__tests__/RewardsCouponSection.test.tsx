import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import RewardsCouponSection from '../RewardsCouponSection';

const mocks = vi.hoisted(() => ({
  mockUseCoreState: vi.fn(),
  mockUseUser: vi.fn(),
  mockCreditsApi: { getBalance: vi.fn(), getUserCoupons: vi.fn(), redeemCoupon: vi.fn() },
}));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => mocks.mockUseCoreState(),
}));

vi.mock('../../../hooks/useUser', () => ({ useUser: () => mocks.mockUseUser() }));

vi.mock('../../../services/api/creditsApi', () => ({ creditsApi: mocks.mockCreditsApi }));

describe('RewardsCouponSection', () => {
  const refetch = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.mockUseCoreState.mockReturnValue({ snapshot: { sessionToken: 'test-token' } });
    mocks.mockUseUser.mockReturnValue({ refetch });
  });

  it('loads balances and refreshes history after a successful redemption', async () => {
    mocks.mockCreditsApi.getBalance
      .mockResolvedValueOnce({ promotionBalanceUsd: 3, teamTopupUsd: 1 })
      .mockResolvedValueOnce({ promotionBalanceUsd: 8, teamTopupUsd: 1 });
    mocks.mockCreditsApi.getUserCoupons
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        {
          code: 'APRL-2026',
          amountUsd: 5,
          redeemedAt: '2026-04-09T19:00:00.000Z',
          activationType: 'IMMEDIATE',
          activationCondition: null,
          fulfilled: true,
          fulfilledAt: '2026-04-09T19:00:01.000Z',
        },
      ]);
    mocks.mockCreditsApi.redeemCoupon.mockResolvedValueOnce({
      couponCode: 'APRL-2026',
      amountUsd: 5,
      pending: false,
    });

    render(<RewardsCouponSection />);

    expect(await screen.findByText('$3.00')).toBeInTheDocument();
    expect(screen.getByText('No reward codes redeemed yet.')).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText('Promo code'), { target: { value: 'aprl-2026' } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply code' }));

    expect(
      await screen.findByText('APRL-2026 redeemed. $5.00 was added to your credits.')
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText('$8.00')).toBeInTheDocument();
    });
    expect(screen.getByText('APRL-2026')).toBeInTheDocument();
    expect(screen.getByText('Applied')).toBeInTheDocument();
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it('shows backend redemption errors without clearing the existing state', async () => {
    mocks.mockCreditsApi.getBalance.mockResolvedValue({ promotionBalanceUsd: 3, teamTopupUsd: 0 });
    mocks.mockCreditsApi.getUserCoupons.mockResolvedValue([]);
    mocks.mockCreditsApi.redeemCoupon.mockRejectedValueOnce({
      error: 'This coupon has already been used.',
    });

    render(<RewardsCouponSection />);

    expect(await screen.findByText('$3.00')).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText('Promo code'), { target: { value: 'used-code' } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply code' }));

    expect(await screen.findByText('This coupon has already been used.')).toBeInTheDocument();
    expect(mocks.mockCreditsApi.getBalance).toHaveBeenCalledTimes(1);
    expect(refetch).not.toHaveBeenCalled();
  });

  it('shows pending coupon copy and keeps the current balance until the reward is fulfilled', async () => {
    mocks.mockCreditsApi.getBalance
      .mockResolvedValueOnce({ promotionBalanceUsd: 3, teamTopupUsd: 0 })
      .mockResolvedValueOnce({ promotionBalanceUsd: 3, teamTopupUsd: 0 });
    mocks.mockCreditsApi.getUserCoupons
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        {
          code: 'APRL-2026',
          amountUsd: 5,
          redeemedAt: '2026-04-09T19:00:00.000Z',
          activationType: 'CONDITIONAL',
          activationCondition: 'SUBSCRIBE_PAID_PLAN',
          fulfilled: false,
          fulfilledAt: null,
        },
      ]);
    mocks.mockCreditsApi.redeemCoupon.mockResolvedValueOnce({
      couponCode: 'APRL-2026',
      amountUsd: 5,
      pending: true,
    });

    render(<RewardsCouponSection />);

    expect(await screen.findByText('$3.00')).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText('Promo code'), { target: { value: 'aprl-2026' } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply code' }));

    expect(
      await screen.findByText(
        'APRL-2026 accepted. $5.00 will unlock after the required action is completed.'
      )
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getAllByText('$3.00')).toHaveLength(1);
    });
    expect(screen.getByText('APRL-2026')).toBeInTheDocument();
    expect(screen.getByText('Pending action')).toBeInTheDocument();
    expect(refetch).toHaveBeenCalledTimes(1);
  });
});
