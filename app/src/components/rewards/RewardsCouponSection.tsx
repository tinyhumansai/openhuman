import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useUser } from '../../hooks/useUser';
import { useCoreState } from '../../providers/CoreStateProvider';
import {
  type CouponRedeemResult,
  type CreditBalance,
  creditsApi,
  type RedeemedCoupon,
} from '../../services/api/creditsApi';

const log = createDebug('openhuman:rewards-coupons');

function formatUsd(amount: number): string {
  return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(amount);
}

function formatDateTime(value: string | null): string {
  if (!value) return 'Pending';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'Pending' : date.toLocaleString();
}

function redemptionStatus(coupon: RedeemedCoupon): string {
  if (coupon.fulfilled) return 'Applied';
  if (coupon.activationType === 'CONDITIONAL') return 'Pending action';
  return 'Redeemed';
}

function redemptionStatusClass(coupon: RedeemedCoupon): string {
  if (coupon.fulfilled) return 'bg-sage-100 text-sage-700';
  if (coupon.activationType === 'CONDITIONAL') return 'bg-amber-50 text-amber-800';
  return 'bg-stone-100 text-stone-700';
}

function successMessage(result: CouponRedeemResult): string {
  if (result.pending) {
    return `${result.couponCode} accepted. ${formatUsd(result.amountUsd)} will unlock after the required action is completed.`;
  }
  return `${result.couponCode} redeemed. ${formatUsd(result.amountUsd)} was added to your credits.`;
}

const RewardsCouponSection = () => {
  const { snapshot } = useCoreState();
  const { refetch } = useUser();
  const token = snapshot.sessionToken;

  const [couponCode, setCouponCode] = useState('');
  const [creditBalance, setCreditBalance] = useState<CreditBalance | null>(null);
  const [redeemedCoupons, setRedeemedCoupons] = useState<RedeemedCoupon[]>([]);
  const [loading, setLoading] = useState(false);
  const [submitLoading, setSubmitLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitSuccess, setSubmitSuccess] = useState<string | null>(null);
  const latestRequestIdRef = useRef(0);

  const loadCouponState = useCallback(async () => {
    if (!token) {
      latestRequestIdRef.current += 1;
      setCreditBalance(null);
      setRedeemedCoupons([]);
      setLoadError(null);
      setLoading(false);
      return;
    }

    latestRequestIdRef.current += 1;
    const requestId = latestRequestIdRef.current;
    setLoading(true);
    setLoadError(null);

    try {
      log('[load] fetching balance and coupon history');
      const [balance, coupons] = await Promise.all([
        creditsApi.getBalance(),
        creditsApi.getUserCoupons(),
      ]);

      if (requestId !== latestRequestIdRef.current) return;

      log('[load] loaded balance=%O coupons=%d', balance, coupons.length);
      setCreditBalance(balance);
      setRedeemedCoupons(coupons);
    } catch (error) {
      if (requestId !== latestRequestIdRef.current) return;
      const message =
        error && typeof error === 'object' && 'error' in error
          ? String((error as { error: unknown }).error)
          : 'Could not load reward codes right now.';
      log('[load] failed: %s', message);
      setLoadError(message);
    } finally {
      if (requestId === latestRequestIdRef.current) {
        setLoading(false);
      }
    }
  }, [token]);

  useEffect(() => {
    void loadCouponState();
  }, [loadCouponState]);

  const handleRedeem = async () => {
    const code = couponCode.trim();
    if (!code || submitLoading) return;

    setSubmitLoading(true);
    setSubmitError(null);
    setSubmitSuccess(null);

    try {
      log('[redeem] submitting code=%s', code);
      const result = await creditsApi.redeemCoupon(code);
      setSubmitSuccess(successMessage(result));
      setCouponCode('');

      const refreshResults = await Promise.allSettled([loadCouponState(), refetch()]);
      const refreshFailures = refreshResults.filter(
        (result): result is PromiseRejectedResult => result.status === 'rejected'
      );
      if (refreshFailures.length > 0) {
        log('[redeem] refresh failed count=%d', refreshFailures.length);
      }

      log(
        '[redeem] completed code=%s pending=%s amount=%s',
        result.couponCode,
        result.pending,
        result.amountUsd
      );
    } catch (error) {
      const message =
        error && typeof error === 'object' && 'error' in error
          ? String((error as { error: unknown }).error)
          : 'Could not apply that reward code.';
      log('[redeem] failed: %s', message);
      setSubmitError(message);
    } finally {
      setSubmitLoading(false);
    }
  };

  if (!token) {
    return null;
  }

  return (
    <section className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6 space-y-5">
      <div className="space-y-2">
        <div className="inline-flex items-center gap-2 rounded-full border border-sage-200 bg-sage-50 px-3 py-1 text-xs font-medium text-sage-700">
          Promo and reward codes
        </div>
        <h2 className="text-2xl font-semibold text-stone-900">Apply a reward code</h2>
        <p className="max-w-2xl text-sm text-stone-600">
          Redeem promo or campaign codes here. Referral attribution stays in the referral section
          above. Successful redemptions refresh your credits immediately, and pending rewards stay
          visible in your history.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <div className="rounded-xl border border-stone-200 bg-stone-50 p-4">
          <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
            Promo credits
          </div>
          <div className="mt-2 text-2xl font-semibold text-stone-900">
            {creditBalance ? formatUsd(creditBalance.promotionBalanceUsd) : loading ? '…' : '—'}
          </div>
        </div>
        <div className="rounded-xl border border-stone-200 bg-stone-50 p-4">
          <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
            Team top-up
          </div>
          <div className="mt-2 text-2xl font-semibold text-stone-900">
            {creditBalance ? formatUsd(creditBalance.teamTopupUsd) : loading ? '…' : '—'}
          </div>
        </div>
        <div className="rounded-xl border border-stone-200 bg-stone-50 p-4">
          <div className="text-xs font-medium uppercase tracking-wide text-stone-400">
            Redeemed codes
          </div>
          <div className="mt-2 text-2xl font-semibold text-stone-900">{redeemedCoupons.length}</div>
        </div>
      </div>

      <div className="rounded-xl border border-primary-100 bg-primary-50/40 p-4 space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            type="text"
            value={couponCode}
            onChange={event => {
              setCouponCode(event.target.value.toUpperCase());
              if (submitError) setSubmitError(null);
              if (submitSuccess) setSubmitSuccess(null);
            }}
            onKeyDown={event => {
              if (event.key === 'Enter') {
                void handleRedeem();
              }
            }}
            placeholder="Promo code"
            disabled={submitLoading}
            className="flex-1 px-4 py-2.5 rounded-xl border border-stone-200 bg-white font-mono text-stone-900 placeholder:text-stone-400 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
          />
          <button
            type="button"
            onClick={() => void handleRedeem()}
            disabled={submitLoading || !couponCode.trim()}
            className="rounded-xl bg-primary-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-primary-700 disabled:opacity-50">
            {submitLoading ? 'Applying…' : 'Apply code'}
          </button>
        </div>
        {submitSuccess ? (
          <div className="rounded-xl border border-sage-200 bg-sage-50 px-3 py-2 text-sm text-sage-800">
            {submitSuccess}
          </div>
        ) : null}
        {submitError ? (
          <div className="rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-800">
            {submitError}
          </div>
        ) : null}
        {loadError ? (
          <div className="rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-800">
            {loadError}
            <button
              type="button"
              onClick={() => void loadCouponState()}
              className="ml-2 font-medium underline">
              Retry
            </button>
          </div>
        ) : null}
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <h3 className="text-sm font-semibold text-stone-900">Recent redemptions</h3>
          <button
            type="button"
            onClick={() => void loadCouponState()}
            disabled={loading}
            className="text-xs font-medium text-stone-500 transition-colors hover:text-stone-700 disabled:opacity-50">
            Refresh
          </button>
        </div>

        {loading && redeemedCoupons.length === 0 ? (
          <p className="text-sm text-stone-500">Loading reward history…</p>
        ) : null}

        {redeemedCoupons.length === 0 && !loading && !loadError ? (
          <p className="text-sm text-stone-500 rounded-xl border border-dashed border-stone-200 px-4 py-6 text-center">
            No reward codes redeemed yet.
          </p>
        ) : redeemedCoupons.length > 0 ? (
          <div className="overflow-x-auto rounded-xl border border-stone-200">
            <table className="min-w-full text-sm text-left">
              <thead className="bg-stone-50 text-xs uppercase tracking-wide text-stone-500">
                <tr>
                  <th className="px-3 py-2 font-medium">Code</th>
                  <th className="px-3 py-2 font-medium">Reward</th>
                  <th className="px-3 py-2 font-medium">Status</th>
                  <th className="px-3 py-2 font-medium">Redeemed</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-stone-100">
                {redeemedCoupons.map(coupon => (
                  <tr
                    key={`${coupon.code}-${coupon.redeemedAt ?? coupon.activationType}`}
                    className="bg-white">
                    <td className="px-3 py-2 font-mono text-stone-800">{coupon.code}</td>
                    <td className="px-3 py-2 text-stone-700">{formatUsd(coupon.amountUsd)}</td>
                    <td className="px-3 py-2">
                      <span
                        className={`inline-flex rounded-full px-2.5 py-0.5 text-xs font-medium ${redemptionStatusClass(coupon)}`}>
                        {redemptionStatus(coupon)}
                      </span>
                    </td>
                    <td className="px-3 py-2 text-xs text-stone-500">
                      {formatDateTime(coupon.redeemedAt)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>
    </section>
  );
};

export default RewardsCouponSection;
