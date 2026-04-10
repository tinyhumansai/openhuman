import createDebug from 'debug';
import { useState } from 'react';

import { type CreditBalance, creditsApi } from '../../../../services/api/creditsApi';

const log = createDebug('openhuman:billing-payg');

interface PayAsYouGoCardProps {
  creditBalance: CreditBalance | null;
  isLoadingCredits: boolean;
  isToppingUp: boolean;
  onTopUp: (amountUsd: number) => void;
  onBalanceRefresh: () => void;
}

const PayAsYouGoCard = ({
  creditBalance,
  isLoadingCredits,
  isToppingUp,
  onTopUp,
  onBalanceRefresh,
}: PayAsYouGoCardProps) => {
  // Backend `GET /payments/credits/balance` returns
  //   { promotionBalanceUsd, teamTopupUsd }
  // `promotionBalanceUsd` lives on the user document
  // (`IUserUsage.promotionBalanceUsd`) and unifies signup bonus, coupons,
  // and referral rewards. `teamTopupUsd` is the team-level paid top-up pool.
  // Together they make the pay-as-you-go spendable balance.
  const promoCredits = creditBalance?.promotionBalanceUsd ?? 0;
  const teamTopupCredits = creditBalance?.teamTopupUsd ?? 0;
  const availableCredits = promoCredits + teamTopupCredits;

  // Coupon state (local — no need to share with other sections)
  const [couponCode, setCouponCode] = useState('');
  const [couponLoading, setCouponLoading] = useState(false);
  const [couponError, setCouponError] = useState<string | null>(null);
  const [couponSuccess, setCouponSuccess] = useState<string | null>(null);

  const handleRedeemCoupon = async () => {
    const code = couponCode.trim();
    if (!code || couponLoading) return;

    setCouponLoading(true);
    setCouponError(null);
    setCouponSuccess(null);

    try {
      log('[coupon] redeeming code=%s', code);
      const result = await creditsApi.redeemCoupon(code);
      setCouponSuccess(
        result.pending
          ? `Coupon accepted. $${result.amountUsd.toFixed(2)} will be added after the required action.`
          : `Coupon redeemed! $${result.amountUsd.toFixed(2)} added to your credits.`
      );
      setCouponCode('');
      onBalanceRefresh();
    } catch (err) {
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: unknown }).error)
          : 'Invalid or expired coupon code.';
      log('[coupon] error: %s', msg);
      setCouponError(msg);
    } finally {
      setCouponLoading(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-3">
      <h3 className="text-sm font-semibold text-stone-900 mb-2">Pay as You Go</h3>

      {/* Balance display */}
      {creditBalance ? (
        <div className="space-y-1.5 mb-3">
          <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-2.5 py-2">
            <span className="text-xs text-stone-500">Available credits</span>
            <span className="text-sm font-semibold text-stone-900">
              ${availableCredits.toFixed(2)}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span className="text-xs text-stone-400">Signup + promo credits</span>
            <span className="text-xs font-medium text-stone-900">${promoCredits.toFixed(2)}</span>
          </div>
          <div className="flex items-center justify-between">
            <span className="text-xs text-stone-400">Team top-up credits</span>
            <span className="text-xs font-medium text-stone-900">
              ${teamTopupCredits.toFixed(2)}
            </span>
          </div>
        </div>
      ) : isLoadingCredits ? (
        <div className="space-y-1.5 mb-3">
          <div className="h-3 w-full rounded bg-stone-700/60 animate-pulse" />
          <div className="h-3 w-3/4 rounded bg-stone-700/60 animate-pulse" />
        </div>
      ) : (
        <p className="text-xs text-stone-500 mb-3">Unable to load balance</p>
      )}

      <p className="mb-3 text-[11px] text-stone-500">
        No subscription needed. Free users spend from any signup or promo credit first, then from
        top-ups. Paid plans still consume included budget before pay-as-you-go credits.
      </p>

      {/* Top-up buttons */}
      <div className="flex gap-2 mb-3">
        {[5, 10, 25].map(amount => (
          <button
            key={amount}
            onClick={() => onTopUp(amount)}
            disabled={isToppingUp}
            className="flex-1 py-1.5 rounded-lg bg-primary-500/20 hover:bg-primary-500/30 text-primary-400 text-xs font-medium border border-primary-500/30 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
            {isToppingUp ? '…' : `+$${amount}`}
          </button>
        ))}
      </div>

      {/* Coupon redemption */}
      <div className="border-t border-stone-100 pt-3">
        <p className="text-[11px] text-stone-400 mb-1.5">Have a coupon?</p>
        <div className="flex gap-2">
          <input
            type="text"
            value={couponCode}
            onChange={e => {
              setCouponCode(e.target.value.toUpperCase());
              if (couponError) setCouponError(null);
              if (couponSuccess) setCouponSuccess(null);
            }}
            onKeyDown={e => {
              if (e.key === 'Enter') handleRedeemCoupon();
            }}
            placeholder="XXXX-XXXX"
            className="flex-1 px-2.5 py-1.5 text-xs rounded-lg border border-stone-200 bg-stone-50 text-stone-900 placeholder-stone-400 focus:outline-none focus:ring-1 focus:ring-primary-500 focus:border-primary-500"
          />
          <button
            onClick={handleRedeemCoupon}
            disabled={couponLoading || !couponCode.trim()}
            className="px-3 py-1.5 text-xs font-medium rounded-lg transition-colors bg-primary-500 hover:bg-primary-600 text-white disabled:opacity-50 disabled:cursor-not-allowed">
            {couponLoading ? '…' : 'Redeem'}
          </button>
        </div>

        {couponSuccess && (
          <div className="mt-2 flex items-center gap-1.5 rounded-lg bg-sage-500/10 border border-sage-500/20 px-2.5 py-1.5">
            <svg
              className="w-3.5 h-3.5 text-sage-400 flex-shrink-0"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M5 13l4 4L19 7"
              />
            </svg>
            <p className="text-[11px] text-sage-300 font-medium">{couponSuccess}</p>
          </div>
        )}

        {couponError && (
          <div className="mt-2 flex items-center gap-1.5 rounded-lg bg-coral-500/10 border border-coral-500/20 px-2.5 py-1.5">
            <svg
              className="w-3.5 h-3.5 text-coral-400 flex-shrink-0"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
              />
            </svg>
            <p className="text-[11px] text-coral-300">{couponError}</p>
          </div>
        )}
      </div>
    </div>
  );
};

export default PayAsYouGoCard;
