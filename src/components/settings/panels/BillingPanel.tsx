import { useCallback, useEffect, useRef, useState } from 'react';

import { billingApi } from '../../../services/api/billingApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { fetchCurrentUser } from '../../../store/userSlice';
import type { PlanTier } from '../../../types/api';
import { openUrl } from '../../../utils/openUrl';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  annualSavings,
  buildPlanId,
  isUpgrade as checkIsUpgrade,
  displayPrice,
  PLANS,
} from './billingHelpers';

// ── Component ───────────────────────────────────────────────────────────
const BillingPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const { teams } = useAppSelector(state => state.team);

  // Active team context
  const activeTeamId = user?.activeTeamId;
  const activeTeam = teams.find(t => t.team._id === activeTeamId);
  const teamName = activeTeam?.team.name;

  // Derive plan from active team (team is source of truth)
  const currentTier: PlanTier = activeTeam?.team.subscription?.plan ?? 'FREE';
  const hasActive = activeTeam?.team.subscription?.hasActiveSubscription ?? false;
  const planExpiry = activeTeam?.team.subscription?.planExpiry;
  const usage = user?.usage;

  // Local state
  const [billingInterval, setBillingInterval] = useState<'monthly' | 'annual'>('monthly');
  const [paymentMethod, setPaymentMethod] = useState<'card' | 'crypto'>('card');
  const [isPurchasing, setIsPurchasing] = useState(false);
  const [purchasingTier, setPurchasingTier] = useState<PlanTier | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pollStartRef = useRef<number>(0);

  // Fetch current plan on mount
  useEffect(() => {
    billingApi.getCurrentPlan().catch(console.error);
  }, []);

  // When crypto is selected, force annual
  useEffect(() => {
    if (paymentMethod === 'crypto') {
      setBillingInterval('annual');
    }
  }, [paymentMethod]);

  // Cleanup poll on unmount
  useEffect(() => {
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  // ── Poll for plan change after checkout ─────────────────────────────
  const currentTierRef = useRef(currentTier);
  useEffect(() => {
    currentTierRef.current = currentTier;
  }, [currentTier]);

  // eslint-disable-next-line react-hooks/preserve-manual-memoization
  const startPolling = useCallback(() => {
    if (pollRef.current) clearInterval(pollRef.current);
    pollStartRef.current = Date.now();

    pollRef.current = setInterval(async () => {
      // Stop after 2 minutes
      if (Date.now() - pollStartRef.current > 120_000) {
        if (pollRef.current) clearInterval(pollRef.current);
        setIsPurchasing(false);
        setPurchasingTier(null);
        return;
      }

      try {
        const plan = await billingApi.getCurrentPlan();
        if (plan.hasActiveSubscription && plan.plan !== currentTierRef.current) {
          dispatch(fetchCurrentUser());
          setIsPurchasing(false);
          setPurchasingTier(null);
          if (pollRef.current) clearInterval(pollRef.current);
        }
      } catch {
        // Ignore polling errors
      }
    }, 5_000);
  }, [dispatch]);

  // ── Purchase handlers ───────────────────────────────────────────────
  const handleUpgrade = async (tier: PlanTier) => {
    if (tier === 'FREE' || tier === currentTier) return;
    setIsPurchasing(true);
    setPurchasingTier(tier);

    try {
      if (paymentMethod === 'crypto') {
        const { hostedUrl } = await billingApi.createCoinbaseCharge(tier, 'annual');
        await openUrl(hostedUrl);
      } else {
        const planId = buildPlanId(tier, billingInterval);
        const { checkoutUrl } = await billingApi.purchasePlan(planId);
        if (checkoutUrl) await openUrl(checkoutUrl);
      }
      startPolling();
    } catch (err) {
      console.error('Purchase failed:', err);
      setIsPurchasing(false);
      setPurchasingTier(null);
    }
  };

  const handleManageSubscription = async () => {
    try {
      const { portalUrl } = await billingApi.createPortalSession();
      await openUrl(portalUrl);
    } catch (err) {
      console.error('Portal session failed:', err);
    }
  };

  // ── JSX ─────────────────────────────────────────────────────────────
  return (
    <div className="overflow-hidden flex flex-col">
      <SettingsHeader
        title={teamName ? `Billing — ${teamName}` : 'Billing & Subscription'}
        showBackButton={true}
        onBack={navigateBack}
      />

      {/* <div className="flex items-center justify-between max-w-md mx-auto"> */}
      <div className="overflow-y-auto">
        <div className="space-y-2">
          <div className="max-w-md mt-4 mx-auto">
            <div className="p-2.5">
              <div className="flex items-center justify-between mb-1.5">
                <h3 className="text-sm font-semibold text-white">
                  Your Current Plan {currentTier}
                </h3>
                {usage && (
                  <span className="text-xs text-stone-400">
                    {Math.round((usage.spentThisCycleUsd / usage.cycleBudgetUsd) * 100)}% used
                  </span>
                )}
              </div>

              {hasActive && (
                <div className="flex items-center justify-between mb-1.5">
                  {planExpiry && (
                    <p className="text-xs text-stone-400">
                      Renews{' '}
                      {new Date(planExpiry).toLocaleDateString('en-US', {
                        month: 'long',
                        day: 'numeric',
                        year: 'numeric',
                      })}
                    </p>
                  )}
                  <button
                    onClick={handleManageSubscription}
                    className="text-xs text-primary-400 hover:text-primary-300 font-medium transition-colors">
                    Manage Subscription
                  </button>
                </div>
              )}
              {/* Renewal date (for non-active subscriptions) */}
              {!hasActive && planExpiry && (
                <p className="text-xs text-stone-400 mb-1.5">
                  Renews{' '}
                  {new Date(planExpiry).toLocaleDateString('en-US', {
                    month: 'long',
                    day: 'numeric',
                    year: 'numeric',
                  })}
                </p>
              )}
              {usage && (
                <div className="h-1.5 bg-stone-700/60 rounded-full overflow-hidden">
                  <div
                    className="h-full rounded-full transition-all duration-300 bg-primary-500"
                    style={{
                      width: `${Math.min(
                        100,
                        (usage.spentThisCycleUsd / usage.cycleBudgetUsd) * 100
                      )}%`,
                    }}
                  />
                </div>
              )}
            </div>
          </div>

          {/* ── Interval toggle ──────────────────────────────────── */}
          <div className="flex items-center justify-center gap-2 px-4">
            <button
              onClick={() => {
                if (paymentMethod !== 'crypto') setBillingInterval('monthly');
              }}
              disabled={paymentMethod === 'crypto'}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
                billingInterval === 'monthly'
                  ? 'bg-primary-500/20 text-primary-400 border border-primary-500/30'
                  : 'text-stone-400 hover:text-stone-300'
              } ${paymentMethod === 'crypto' ? 'opacity-40 cursor-not-allowed' : ''}`}>
              Monthly
            </button>
            <button
              onClick={() => setBillingInterval('annual')}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
                billingInterval === 'annual'
                  ? 'bg-primary-500/20 text-primary-400 border border-primary-500/30'
                  : 'text-stone-400 hover:text-stone-300'
              }`}>
              Annual
            </button>
          </div>

          <div className="max-w-md mx-auto">
            {/* ── Plan tier cards ───────────────────────────────────── */}
            <div className="space-y-2 px-4">
              {PLANS.map(plan => {
                const isCurrent = plan.tier === currentTier;
                const isUpgrade = checkIsUpgrade(plan.tier, currentTier);
                const savings = annualSavings(plan, billingInterval);
                const isThisPurchasing = isPurchasing && purchasingTier === plan.tier;

                return (
                  <div
                    key={plan.tier}
                    className={`rounded-2xl border p-3 transition-all ${
                      isCurrent
                        ? 'border-primary-500/40 bg-primary-500/5'
                        : 'border-stone-700/50 bg-stone-800/40'
                    }`}>
                    <div className="flex items-start justify-between mb-2">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 flex-wrap">
                          <h4 className="text-sm font-semibold text-white">{plan.name}</h4>
                          {/* Features inline with title */}
                          {plan.features.map(f => (
                            <span key={f.text} className="text-xs text-stone-300">
                              <span className="text-stone-500 mx-1">•</span>
                              {f.text}
                            </span>
                          ))}
                          {isCurrent && (
                            <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-primary-500/20 text-primary-400 border border-primary-500/30">
                              Current
                            </span>
                          )}
                          {savings && (
                            <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-400 border border-sage-500/30">
                              Save {savings}%
                            </span>
                          )}
                        </div>
                        <div className="mt-0.5 flex items-baseline gap-1">
                          <span className="text-xl font-bold text-white">
                            {displayPrice(plan, billingInterval)}
                          </span>
                          {plan.tier !== 'FREE' && (
                            <span className="text-xs text-stone-400">/mo</span>
                          )}
                          {plan.tier !== 'FREE' && billingInterval === 'annual' && (
                            <span className="text-xs text-stone-500 ml-1">
                              (billed ${plan.annualPrice}/yr)
                            </span>
                          )}
                        </div>
                      </div>

                      {/* Action button */}
                      {isUpgrade && (
                        <button
                          onClick={() => handleUpgrade(plan.tier)}
                          disabled={isPurchasing}
                          className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors flex-shrink-0 ${
                            isPurchasing
                              ? 'bg-stone-700/40 text-stone-500 cursor-not-allowed'
                              : 'bg-primary-500 hover:bg-primary-600 text-white'
                          }`}>
                          {isThisPurchasing ? 'Waiting...' : 'Upgrade'}
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>

            {/* ── Purchasing overlay message ────────────────────────── */}
            {isPurchasing && (
              <div className="rounded-xl bg-amber-500/10 border border-amber-500/20 p-3 mx-4">
                <div className="flex items-center gap-2">
                  <svg
                    className="w-4 h-4 text-amber-400 animate-spin"
                    fill="none"
                    viewBox="0 0 24 24">
                    <circle
                      className="opacity-25"
                      cx="12"
                      cy="12"
                      r="10"
                      stroke="currentColor"
                      strokeWidth="4"
                    />
                    <path
                      className="opacity-75"
                      fill="currentColor"
                      d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                    />
                  </svg>
                  <p className="text-xs text-amber-300">
                    Waiting for payment confirmation... Complete checkout in the browser window that
                    opened.
                  </p>
                </div>
              </div>
            )}

            {/* ── Pay with crypto toggle ────────────────────────────── */}
            <div className="flex items-center justify-between rounded-xl bg-stone-800/40 border border-stone-700/40 p-3 mx-4">
              <div>
                <p className="text-xs font-medium text-white">Pay with Crypto</p>
                <p className="text-[11px] text-stone-400 mt-0.5">
                  You can choose to pay annually using crypto
                </p>
              </div>
              <button
                onClick={() => setPaymentMethod(m => (m === 'card' ? 'crypto' : 'card'))}
                className={`relative w-10 h-5 rounded-full transition-colors ${
                  paymentMethod === 'crypto' ? 'bg-primary-500' : 'bg-stone-600'
                }`}
                role="switch"
                aria-checked={paymentMethod === 'crypto'}>
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                    paymentMethod === 'crypto' ? 'translate-x-5' : 'translate-x-0'
                  }`}
                />
              </button>
            </div>

            {/* ── Upgrade benefits ───────────────────────────────────── */}
            <div className="px-4 pb-4 pt-2">
              <div className="rounded-xl bg-gradient-to-br from-primary-500/10 to-sage-500/10 border border-primary-500/20 p-4">
                <h3 className="text-sm font-semibold text-white mb-2">Why upgrade?</h3>
                <ul className="space-y-1.5 text-xs text-stone-300">
                  <li className="flex items-start gap-2">
                    <svg
                      className="w-4 h-4 text-sage-400 flex-shrink-0 mt-0.5"
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
                    <span>Unlock higher daily limits for more AI interactions</span>
                  </li>
                  {currentTier === 'FREE' && (
                    <li className="flex items-start gap-2">
                      <svg
                        className="w-4 h-4 text-sage-400 flex-shrink-0 mt-0.5"
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
                      <span>
                        Save up to 20% with annual plans and never worry about hitting limits
                      </span>
                    </li>
                  )}
                </ul>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default BillingPanel;
