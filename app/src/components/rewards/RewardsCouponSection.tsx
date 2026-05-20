import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useUser } from '../../hooks/useUser';
import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { type CreditBalance, creditsApi, type RedeemedCoupon } from '../../services/api/creditsApi';

const log = createDebug('openhuman:rewards-coupons');

function formatUsd(amount: number): string {
  return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(amount);
}

function formatDateTime(value: string | null, pendingLabel: string): string {
  if (!value) return pendingLabel;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? pendingLabel : date.toLocaleString();
}

function redemptionStatusClass(coupon: RedeemedCoupon): string {
  if (coupon.fulfilled) return 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300';
  if (coupon.activationType === 'CONDITIONAL')
    return 'bg-amber-50 dark:bg-amber-500/10 text-amber-800 dark:text-amber-200';
  return 'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200';
}

const RewardsCouponSection = () => {
  const { t } = useT();
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
      const successMsg = result.pending
        ? t('rewards.coupon.redeemAccepted')
            .replace('{code}', result.couponCode)
            .replace('{amount}', formatUsd(result.amountUsd))
        : t('rewards.coupon.redeemSuccess')
            .replace('{code}', result.couponCode)
            .replace('{amount}', formatUsd(result.amountUsd));
      setSubmitSuccess(successMsg);
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
    <>
      <section className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6 space-y-5">
        <div className="space-y-2">
          <h2 className="text-2xl font-semibold text-stone-900 dark:text-neutral-100">
            {t('rewards.coupon.title')}
          </h2>
          <p className="max-w-2xl text-sm text-stone-600 dark:text-neutral-300">
            {t('rewards.coupon.subtitle')}
          </p>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
            <div className="text-xs font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              {t('rewards.coupon.promoCredits')}
            </div>
            <div className="mt-2 text-2xl font-semibold text-stone-900 dark:text-neutral-100">
              {creditBalance ? formatUsd(creditBalance.promotionBalanceUsd) : loading ? '…' : '—'}
            </div>
          </div>
          <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
            <div className="text-xs font-medium uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              {t('rewards.coupon.redeemedCodes')}
            </div>
            <div className="mt-2 text-2xl font-semibold text-stone-900 dark:text-neutral-100">
              {redeemedCoupons.length}
            </div>
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
              placeholder={t('rewards.coupon.placeholder')}
              disabled={submitLoading}
              className="flex-1 px-4 py-2.5 rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 font-mono text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:text-neutral-500 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40"
            />
            <button
              type="button"
              onClick={() => void handleRedeem()}
              disabled={submitLoading || !couponCode.trim()}
              className="rounded-xl bg-primary-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-primary-700 disabled:opacity-50">
              {submitLoading ? t('rewards.coupon.redeeming') : t('rewards.coupon.redeemButton')}
            </button>
          </div>
          {submitSuccess ? (
            <div className="rounded-xl border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-3 py-2 text-sm text-sage-800 dark:text-sage-200">
              {submitSuccess}
            </div>
          ) : null}
          {submitError ? (
            <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-sm text-coral-800 dark:text-coral-200">
              {submitError}
            </div>
          ) : null}
          {loadError ? (
            <div className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-sm text-coral-800 dark:text-coral-200">
              {loadError}
              <button
                type="button"
                onClick={() => void loadCouponState()}
                className="ml-2 font-medium underline">
                {t('common.retry')}
              </button>
            </div>
          ) : null}
        </div>
      </section>
      <section className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6 space-y-5">
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
              {t('rewards.coupon.recentRedemptions')}
            </h3>
            <button
              type="button"
              onClick={() => void loadCouponState()}
              disabled={loading}
              className="text-xs font-medium text-stone-500 dark:text-neutral-400 transition-colors hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 dark:text-neutral-200 disabled:opacity-50">
              {t('common.refresh')}
            </button>
          </div>

          {loading && redeemedCoupons.length === 0 ? (
            <p className="text-sm text-stone-500 dark:text-neutral-400">
              {t('rewards.coupon.loadingHistory')}
            </p>
          ) : null}

          {redeemedCoupons.length === 0 && !loading && !loadError ? (
            <p className="text-sm text-stone-500 dark:text-neutral-400 rounded-xl border border-dashed border-stone-200 dark:border-neutral-800 px-4 py-6 text-center">
              {t('rewards.coupon.noCodes')}
            </p>
          ) : redeemedCoupons.length > 0 ? (
            <div className="overflow-x-auto rounded-xl border border-stone-200 dark:border-neutral-800">
              <table className="min-w-full text-sm text-left">
                <thead className="bg-stone-50 dark:bg-neutral-800/60 text-xs uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                  <tr>
                    <th className="px-3 py-2 font-medium">{t('rewards.coupon.colCode')}</th>
                    <th className="px-3 py-2 font-medium">{t('rewards.coupon.colReward')}</th>
                    <th className="px-3 py-2 font-medium">{t('rewards.coupon.colStatus')}</th>
                    <th className="px-3 py-2 font-medium">{t('rewards.coupon.colRedeemed')}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-stone-100 dark:divide-neutral-800">
                  {redeemedCoupons.map(coupon => (
                    <tr
                      key={`${coupon.code}-${coupon.redeemedAt ?? coupon.activationType}`}
                      className="bg-white dark:bg-neutral-900">
                      <td className="px-3 py-2 font-mono text-stone-800 dark:text-neutral-100">
                        {coupon.code}
                      </td>
                      <td className="px-3 py-2 text-stone-700 dark:text-neutral-200">
                        {formatUsd(coupon.amountUsd)}
                      </td>
                      <td className="px-3 py-2">
                        <span
                          className={`inline-flex rounded-full px-2.5 py-0.5 text-xs font-medium ${redemptionStatusClass(coupon)}`}>
                          {coupon.fulfilled
                            ? t('rewards.coupon.statusApplied')
                            : coupon.activationType === 'CONDITIONAL'
                              ? t('rewards.coupon.statusPendingAction')
                              : t('rewards.coupon.statusRedeemed')}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-xs text-stone-500 dark:text-neutral-400">
                        {formatDateTime(coupon.redeemedAt, t('rewards.coupon.pending'))}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </div>
      </section>
    </>
  );
};

export default RewardsCouponSection;
