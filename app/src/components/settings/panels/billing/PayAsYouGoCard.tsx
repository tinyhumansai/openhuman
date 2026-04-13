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
    <div className="rounded-[28px] bg-white p-6 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
      <div className="flex items-end justify-between gap-3">
        <div>
          <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
            Credits & Add-ons
          </h3>
          <p className="mt-1 text-sm leading-relaxed text-stone-500">
            Top up spendable credits and redeem codes in one place.
          </p>
        </div>
        <div className="text-right">
          <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
            Current balance
          </p>
          <p className="mt-1 text-lg font-bold tracking-tight text-primary-600">
            ${availableCredits.toFixed(2)}
          </p>
        </div>
      </div>

      {creditBalance ? (
        <div className="mt-5 grid gap-3 sm:grid-cols-3">
          <div className="rounded-2xl bg-stone-50 px-4 py-4">
            <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Available
            </p>
            <p className="mt-2 text-2xl font-bold tracking-tight text-stone-950">
              ${availableCredits.toFixed(2)}
            </p>
          </div>
          <div className="rounded-2xl bg-stone-50 px-4 py-4">
            <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Promo
            </p>
            <p className="mt-2 text-xl font-bold tracking-tight text-stone-950">
              ${promoCredits.toFixed(2)}
            </p>
          </div>
          <div className="rounded-2xl bg-stone-50 px-4 py-4">
            <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Top-up pool
            </p>
            <p className="mt-2 text-xl font-bold tracking-tight text-stone-950">
              ${teamTopupCredits.toFixed(2)}
            </p>
          </div>
        </div>
      ) : isLoadingCredits ? (
        <div className="mt-5 grid gap-3 sm:grid-cols-3">
          {[0, 1, 2].map(index => (
            <div key={index} className="h-24 rounded-2xl bg-stone-100 animate-pulse" />
          ))}
        </div>
      ) : (
        <p className="mt-5 text-sm text-stone-500">Unable to load balance.</p>
      )}

      <div className="mt-6 grid gap-3 sm:grid-cols-3">
        {[5, 10, 25].map(amount => (
          <button
            key={amount}
            onClick={() => onTopUp(amount)}
            disabled={isToppingUp}
            className="group rounded-2xl border border-stone-200 bg-stone-50 px-4 py-5 text-center transition-all hover:border-primary-200 hover:bg-white disabled:cursor-not-allowed disabled:opacity-50">
            <div className="text-3xl font-bold tracking-tight text-stone-950">+{amount}</div>
            <div className="mt-1 text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Credits
            </div>
            <div className="mt-4 text-sm font-bold text-primary-600 transition-transform group-hover:-translate-y-0.5">
              {isToppingUp ? 'Opening…' : `$${amount.toFixed(2)}`}
            </div>
          </button>
        ))}
      </div>

      <div className="mt-6 grid gap-3 lg:grid-cols-[1fr_auto]">
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
          placeholder="Redeem coupon or code"
          className="w-full rounded-2xl border-0 bg-stone-100 px-5 py-4 text-sm text-stone-900 placeholder:text-stone-400 focus:bg-white focus:outline-none focus:ring-2 focus:ring-primary-500/20"
        />
        <button
          onClick={handleRedeemCoupon}
          disabled={couponLoading || !couponCode.trim()}
          className="rounded-2xl bg-stone-950 px-6 py-4 text-sm font-semibold text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50">
          {couponLoading ? 'Redeeming…' : 'Redeem'}
        </button>
      </div>

      <p className="mt-3 text-sm text-stone-500">
        Credits are consumed after any included subscription budget is exhausted.
      </p>

      {couponSuccess && (
        <div className="mt-4 flex items-center gap-2 rounded-2xl border border-sage-500/20 bg-sage-500/10 px-4 py-3">
          <svg
            className="h-4 w-4 flex-shrink-0 text-sage-500"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
          </svg>
          <p className="text-sm font-medium text-sage-700">{couponSuccess}</p>
        </div>
      )}

      {couponError && (
        <div className="mt-4 flex items-center gap-2 rounded-2xl border border-coral-500/20 bg-coral-500/10 px-4 py-3">
          <svg
            className="h-4 w-4 flex-shrink-0 text-coral-500"
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
          <p className="text-sm text-coral-700">{couponError}</p>
        </div>
      )}
    </div>
  );
};

export default PayAsYouGoCard;
