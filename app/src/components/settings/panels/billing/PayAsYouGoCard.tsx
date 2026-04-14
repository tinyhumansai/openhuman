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
  const [customTopUpAmount, setCustomTopUpAmount] = useState('');

  const parsedCustomTopUpAmount = Number(customTopUpAmount);
  const customTopUpAmountValid =
    customTopUpAmount.trim() !== '' &&
    Number.isFinite(parsedCustomTopUpAmount) &&
    parsedCustomTopUpAmount > 0;

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

  const handleCustomTopUp = () => {
    if (!customTopUpAmountValid || isToppingUp) return;
    onTopUp(parsedCustomTopUpAmount);
  };

  return (
    <div className="rounded-[28px] bg-white p-6 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
      {creditBalance ? (
        <div className="mt-5 grid gap-3 sm:grid-cols-3">
          <div>
            <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Available
            </p>
            <p className="mt-2 text-2xl font-bold tracking-tight text-stone-950">
              ${availableCredits.toFixed(2)}
            </p>
          </div>
          <div>
            <p className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Promo
            </p>
            <p className="mt-2 text-xl font-bold tracking-tight text-stone-950">
              ${promoCredits.toFixed(2)}
            </p>
          </div>
          <div>
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
            className="group rounded-2xl border border-primary-200/50 bg-primary-50/50 px-4 py-5 text-center transition-all hover:border-primary-200 disabled:cursor-not-allowed disabled:opacity-50">
            <div className="text-2xl font-bold tracking-tight text-primary-600">
              {isToppingUp ? 'Opening…' : `$${amount.toFixed(2)}`}
            </div>
            <div className="mt-1 text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
              Credits
            </div>
          </button>
        ))}
      </div>
    </div>
  );
};

export default PayAsYouGoCard;
