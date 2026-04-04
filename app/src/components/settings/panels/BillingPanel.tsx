import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { billingApi } from '../../../services/api/billingApi';
import {
  type AutoRechargeSettings,
  type CreditBalance,
  creditsApi,
  type SavedCard,
  type TeamUsage,
} from '../../../services/api/creditsApi';
import type { CurrentPlanData, PlanTier } from '../../../types/api';
import { openUrl } from '../../../utils/openUrl';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  annualSavings,
  buildPlanId,
  isUpgrade as checkIsUpgrade,
  displayPrice,
  formatStorageLimit,
  formatUsdAmount,
  getPlanMeta,
  PLANS,
} from './billingHelpers';

// ── Constants ────────────────────────────────────────────────────────────────
const log = createDebug('openhuman:billing-panel');
const THRESHOLD_OPTIONS = [5, 10, 20] as const;
const RECHARGE_OPTIONS = [10, 20, 50, 100] as const;
const WEEKLY_LIMIT_OPTIONS = [25, 50, 100, 200, 500] as const;

const CARD_BRAND_LABELS: Record<string, string> = {
  visa: 'Visa',
  mastercard: 'Mastercard',
  amex: 'Amex',
  discover: 'Discover',
  jcb: 'JCB',
  diners: 'Diners',
  unionpay: 'UnionPay',
};

function cardBrandLabel(brand: string) {
  return CARD_BRAND_LABELS[brand.toLowerCase()] ?? brand.charAt(0).toUpperCase() + brand.slice(1);
}

// ── Component ───────────────────────────────────────────────────────────
const BillingPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const { snapshot, teams, refresh } = useCoreState();
  const user = snapshot.currentUser;

  // Active team context
  const activeTeamId = user?.activeTeamId;
  const activeTeam = teams.find(t => t.team._id === activeTeamId);
  const teamName = activeTeam?.team.name;

  // Credits & usage state
  const [currentPlan, setCurrentPlan] = useState<CurrentPlanData | null>(null);
  const [creditBalance, setCreditBalance] = useState<CreditBalance | null>(null);
  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [isLoadingCredits, setIsLoadingCredits] = useState(false);
  const [isToppingUp, setIsToppingUp] = useState(false);

  // Local state
  const [billingInterval, setBillingInterval] = useState<'monthly' | 'annual'>('monthly');
  const [paymentMethod, setPaymentMethod] = useState<'card' | 'crypto'>('card');
  const [isPurchasing, setIsPurchasing] = useState(false);
  const [purchasingTier, setPurchasingTier] = useState<PlanTier | null>(null);
  const [paymentConfirmed, setPaymentConfirmed] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pollStartRef = useRef<number>(0);
  const timeoutRef = useRef<number | null>(null);

  // ── Auto-recharge state ──────────────────────────────────────────────────
  const [arSettings, setArSettings] = useState<AutoRechargeSettings | null>(null);
  const [arLoading, setArLoading] = useState(true);
  const [arError, setArError] = useState<string | null>(null);
  const [arSaving, setArSaving] = useState(false);
  const [arThreshold, setArThreshold] = useState(5);
  const [arAmount, setArAmount] = useState(20);
  const [arWeeklyLimit, setArWeeklyLimit] = useState(50);
  const [arDirty, setArDirty] = useState(false);

  // Recompute dirty flag whenever local settings or server settings change
  useEffect(() => {
    if (!arSettings) return;
    setArDirty(
      arThreshold !== arSettings.thresholdUsd ||
        arAmount !== arSettings.rechargeAmountUsd ||
        arWeeklyLimit !== arSettings.weeklyLimitUsd
    );
  }, [arThreshold, arAmount, arWeeklyLimit, arSettings]);

  // ── Cards state ──────────────────────────────────────────────────────────
  const [cards, setCards] = useState<SavedCard[]>([]);
  const [cardsLoading, setCardsLoading] = useState(true);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [deletingCardId, setDeletingCardId] = useState<string | null>(null);
  const [settingDefaultId, setSettingDefaultId] = useState<string | null>(null);

  const currentTier: PlanTier = currentPlan?.plan ?? activeTeam?.team.subscription?.plan ?? 'FREE';
  const hasActive =
    currentPlan?.hasActiveSubscription ??
    activeTeam?.team.subscription?.hasActiveSubscription ??
    false;
  const planExpiry = currentPlan?.planExpiry ?? activeTeam?.team.subscription?.planExpiry ?? null;
  const currentPlanMeta = getPlanMeta(currentTier);

  // Fetch current plan, credits balance, and team usage on mount
  useEffect(() => {
    setIsLoadingCredits(true);
    Promise.all([billingApi.getCurrentPlan(), creditsApi.getBalance(), creditsApi.getTeamUsage()])
      .then(([plan, balance, usage]) => {
        log(
          '[load] plan=%s active=%s weeklyBudget=%s',
          plan.plan,
          plan.hasActiveSubscription,
          plan.weeklyBudgetUsd
        );
        setCurrentPlan(plan);
        setCreditBalance(balance);
        setTeamUsage(usage);
      })
      .catch(error => {
        log('[load] failed: %O', error);
        console.error(error);
      })
      .finally(() => setIsLoadingCredits(false));
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

  // ── Fetch auto-recharge settings + cards on mount ────────────────────────
  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      try {
        const [settings, cardsData] = await Promise.all([
          creditsApi.getAutoRecharge(),
          creditsApi.getCards(),
        ]);
        if (cancelled) return;
        setArSettings(settings);
        setArThreshold(settings.thresholdUsd);
        setArAmount(settings.rechargeAmountUsd);
        setArWeeklyLimit(settings.weeklyLimitUsd);
        setCards(cardsData.cards);
      } catch {
        if (!cancelled) setArError('Failed to load auto-recharge settings.');
      } finally {
        if (!cancelled) {
          setArLoading(false);
          setCardsLoading(false);
        }
      }
    };

    load().catch(console.error);
    return () => {
      cancelled = true;
    };
  }, []);

  // ── Auto-recharge handlers ───────────────────────────────────────────────
  const handleArToggle = async () => {
    if (!arSettings || arSaving) return;
    const nextEnabled = !arSettings.enabled;

    // Prevent enabling without a saved card
    if (nextEnabled && !arSettings.hasSavedPaymentMethod && cards.length === 0) {
      setArError('Add a payment card before enabling auto-recharge.');
      return;
    }

    setArSaving(true);
    setArError(null);
    try {
      const updated = await creditsApi.updateAutoRecharge({ enabled: nextEnabled });
      setArSettings(updated);
    } catch (err) {
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: unknown }).error)
          : 'Failed to update auto-recharge.';
      setArError(msg);
    } finally {
      setArSaving(false);
    }
  };

  const handleArSave = async () => {
    if (!arSettings || arSaving) return;
    setArSaving(true);
    setArError(null);
    try {
      const updated = await creditsApi.updateAutoRecharge({
        thresholdUsd: arThreshold,
        rechargeAmountUsd: arAmount,
        weeklyLimitUsd: arWeeklyLimit,
      });
      setArSettings(updated);
      setArDirty(false);
    } catch (err) {
      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: unknown }).error)
          : 'Failed to save settings.';
      setArError(msg);
    } finally {
      setArSaving(false);
    }
  };

  // ── Card handlers ────────────────────────────────────────────────────────
  const handleSetDefault = async (paymentMethodId: string) => {
    if (settingDefaultId) return;
    setSettingDefaultId(paymentMethodId);
    try {
      const updated = await creditsApi.updateCard(paymentMethodId, { isDefault: true });
      setCards(updated.cards);
    } catch {
      setArError('Failed to update default card.');
    } finally {
      setSettingDefaultId(null);
    }
  };

  const handleDeleteCard = async (paymentMethodId: string) => {
    if (deletingCardId) return;
    setDeletingCardId(paymentMethodId);
    setConfirmDeleteId(null);
    try {
      const updated = await creditsApi.deleteCard(paymentMethodId);
      setCards(updated.cards);
      // Refresh auto-recharge settings (hasSavedPaymentMethod may change)
      const refreshed = await creditsApi.getAutoRecharge();
      setArSettings(refreshed);
    } catch {
      setArError('Failed to remove card.');
    } finally {
      setDeletingCardId(null);
    }
  };

  const handleAddCard = async () => {
    // Card setup requires Stripe.js confirmation.
    // Use the Stripe Customer Portal as the secure entry point.
    try {
      const { portalUrl } = await billingApi.createPortalSession();
      await openUrl(portalUrl);
    } catch {
      setArError('Could not open payment portal. Please try again.');
    }
  };

  // Handle payment:success deep link event
  useEffect(() => {
    const onPaymentSuccess = async () => {
      // Stop any in-flight poll — we know checkout completed
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      setIsPurchasing(false);
      setPurchasingTier(null);
      setPaymentConfirmed(true);

      // Fetch current plan from backend, then refresh user/teams in store
      try {
        const plan = await billingApi.getCurrentPlan();
        log('[payment-success] plan=%s active=%s', plan.plan, plan.hasActiveSubscription);
        setCurrentPlan(plan);
      } catch (e) {
        console.error('Failed to fetch current plan after payment', e);
      }
      await refresh();

      // Auto-hide the success banner after 5 s
      timeoutRef.current = window.setTimeout(() => setPaymentConfirmed(false), 5_000);
    };

    window.addEventListener('payment:success', onPaymentSuccess);
    return () => {
      window.removeEventListener('payment:success', onPaymentSuccess);
      if (timeoutRef.current !== null) {
        clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
    };
  }, [refresh]);

  // ── Poll for plan change after checkout ─────────────────────────────
  const currentTierRef = useRef(currentTier);
  useEffect(() => {
    currentTierRef.current = currentTier;
  }, [currentTier]);

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
        log('[poll] plan=%s active=%s', plan.plan, plan.hasActiveSubscription);
        setCurrentPlan(plan);
        if (plan.hasActiveSubscription && plan.plan !== currentTierRef.current) {
          await refresh();
          setIsPurchasing(false);
          setPurchasingTier(null);
          if (pollRef.current) clearInterval(pollRef.current);
        }
      } catch {
        // Ignore polling errors
      }
    }, 5_000);
  }, [refresh]);

  // ── Purchase handlers ───────────────────────────────────────────────
  const handleUpgrade = async (tier: PlanTier) => {
    if (tier === 'FREE' || tier === currentTier) return;
    setIsPurchasing(true);
    setPurchasingTier(tier);

    try {
      if (paymentMethod === 'crypto') {
        const { hostedUrl } = await billingApi.createCoinbaseCharge(tier, 'annual');
        log('[purchase] crypto tier=%s', tier);
        await openUrl(hostedUrl);
      } else {
        const planId = buildPlanId(tier, billingInterval);
        const { checkoutUrl } = await billingApi.purchasePlan(planId);
        log('[purchase] stripe planId=%s', planId);
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

  const handleTopUp = async (amountUsd: number) => {
    setIsToppingUp(true);
    try {
      log('[top-up] amountUsd=%s', amountUsd);
      const result = await creditsApi.topUp(amountUsd, 'stripe');
      await openUrl(result.url);
    } catch (err) {
      console.error('Top-up failed:', err);
    } finally {
      setIsToppingUp(false);
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
          <div className="max-w-md mt-4 mx-auto px-4 space-y-3">
            {/* ── Current Plan Header ───────────────────────────────── */}
            <div className="rounded-2xl border border-stone-200 bg-white p-3">
              <div className="flex items-center justify-between mb-1.5">
                <h3 className="text-sm font-semibold text-stone-900">
                  Current Plan — {currentTier}
                </h3>
                {hasActive && (
                  <button
                    onClick={handleManageSubscription}
                    className="text-xs text-primary-400 hover:text-primary-300 font-medium transition-colors">
                    Manage
                  </button>
                )}
              </div>
              {planExpiry && (
                <p className="text-xs text-stone-400 mb-1.5">
                  Renews{' '}
                  {new Date(planExpiry).toLocaleDateString('en-US', {
                    month: 'long',
                    day: 'numeric',
                    year: 'numeric',
                  })}
                </p>
              )}
              <p className="text-xs text-stone-500">
                Your subscription includes premium usage each cycle. Pay-as-you-go credits cover
                overage after the included budget is consumed.
              </p>
              {currentPlan && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    Included monthly value: {formatUsdAmount(currentPlan.monthlyBudgetUsd)}
                  </span>
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    7-day cycle budget: {formatUsdAmount(currentPlan.weeklyBudgetUsd)}
                  </span>
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    5-hour cap: {formatUsdAmount(currentPlan.fiveHourCapUsd)}
                  </span>
                  {currentPlanMeta && (
                    <>
                      <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                        Premium-usage discount: {currentPlanMeta.discountPercent}%
                      </span>
                      <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                        Storage: {formatStorageLimit(currentPlanMeta.storageLimitBytes)}
                      </span>
                    </>
                  )}
                </div>
              )}
            </div>

            {/* ── Inference Budget (Team Usage) ─────────────────────── */}
            <div className="rounded-2xl border border-stone-200 bg-white p-3">
              <div className="flex items-center justify-between mb-2">
                <h3 className="text-sm font-semibold text-stone-900">Inference Budget</h3>
                {isLoadingCredits && <span className="text-[10px] text-stone-500">Loading…</span>}
                {teamUsage && !isLoadingCredits && (
                  <span className="text-xs text-stone-400">
                    ${teamUsage.remainingUsd.toFixed(2)} / ${teamUsage.cycleBudgetUsd.toFixed(2)}{' '}
                    remaining
                  </span>
                )}
              </div>
              {teamUsage ? (
                <>
                  <div className="h-1.5 bg-stone-700/60 rounded-full overflow-hidden mb-2">
                    <div
                      className={`h-full rounded-full transition-all duration-300 ${
                        teamUsage.remainingUsd <= 0
                          ? 'bg-coral-500'
                          : teamUsage.remainingUsd / teamUsage.cycleBudgetUsd < 0.2
                            ? 'bg-amber-500'
                            : 'bg-primary-500'
                      }`}
                      style={{
                        width: `${Math.min(100, (teamUsage.remainingUsd / teamUsage.cycleBudgetUsd) * 100)}%`,
                      }}
                    />
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] text-stone-500">
                      Daily usage: ${teamUsage.dailyUsage.toFixed(3)}
                    </span>
                    <span className="text-[11px] text-stone-500">
                      {(
                        (teamUsage.totalInputTokensThisCycle +
                          teamUsage.totalOutputTokensThisCycle) /
                        1000
                      ).toFixed(1)}
                      k tokens this cycle
                    </span>
                  </div>
                  <div className="mt-1 flex items-center justify-between">
                    <span className="text-[11px] text-stone-500">
                      5-hour cap: ${teamUsage.fiveHourSpendUsd.toFixed(2)} / $
                      {teamUsage.fiveHourCapUsd.toFixed(2)}
                    </span>
                    <span className="text-[11px] text-stone-500">
                      Cycle ends {new Date(teamUsage.cycleEndsAt).toLocaleDateString('en-US')}
                    </span>
                  </div>
                  {teamUsage.remainingUsd <= 0 && (
                    <p className="text-[11px] text-coral-400 mt-1.5">
                      Included subscription usage is exhausted. Top up credits to continue using AI
                      features without waiting for the next cycle.
                    </p>
                  )}
                </>
              ) : isLoadingCredits ? (
                <div className="h-1.5 w-full rounded-full bg-stone-700/60 animate-pulse" />
              ) : (
                <p className="text-xs text-stone-500">Unable to load usage data</p>
              )}
            </div>

            {/* ── Credits Balance & Top-up ──────────────────────────── */}
            <div className="rounded-2xl border border-stone-200 bg-white p-3">
              <h3 className="text-sm font-semibold text-stone-900 mb-2">Pay-as-You-Go Credits</h3>
              {creditBalance ? (
                <div className="space-y-1.5 mb-3">
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-stone-400">General credits</span>
                    <span className="text-xs font-medium text-stone-900">
                      ${creditBalance.balanceUsd.toFixed(2)}
                    </span>
                  </div>
                  <div className="space-y-1">
                    <div className="flex items-center justify-between">
                      <span className="text-xs text-stone-400">Top-up credits</span>
                      <span className="text-xs font-medium text-stone-900">
                        ${creditBalance.topUpBalanceUsd.toFixed(2)}
                        {creditBalance.topUpBaselineUsd != null &&
                          creditBalance.topUpBaselineUsd > 0 && (
                            <span className="text-stone-500 font-normal">
                              {' '}
                              / ${creditBalance.topUpBaselineUsd.toFixed(2)}
                            </span>
                          )}
                      </span>
                    </div>
                    {creditBalance.topUpBaselineUsd != null &&
                      creditBalance.topUpBaselineUsd > 0 && (
                        <div className="h-1 bg-stone-700/60 rounded-full overflow-hidden">
                          <div
                            className={`h-full rounded-full transition-all duration-300 ${
                              creditBalance.topUpBalanceUsd <= 0
                                ? 'bg-coral-500'
                                : creditBalance.topUpBalanceUsd / creditBalance.topUpBaselineUsd <
                                    0.2
                                  ? 'bg-amber-500'
                                  : 'bg-primary-500'
                            }`}
                            style={{
                              width: `${Math.min(
                                100,
                                (creditBalance.topUpBalanceUsd / creditBalance.topUpBaselineUsd) *
                                  100
                              )}%`,
                            }}
                          />
                        </div>
                      )}
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
                Subscription usage is consumed first. Top-up credits are reserved for overflow
                inference, bandwidth, and integration usage.
              </p>
              <div className="flex gap-2">
                {[5, 10, 25].map(amount => (
                  <button
                    key={amount}
                    onClick={() => handleTopUp(amount)}
                    disabled={isToppingUp}
                    className="flex-1 py-1.5 rounded-lg bg-primary-500/20 hover:bg-primary-500/30 text-primary-400 text-xs font-medium border border-primary-500/30 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                    {isToppingUp ? '…' : `+$${amount}`}
                  </button>
                ))}
              </div>
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
                  : 'text-stone-500 hover:text-stone-700'
              } ${paymentMethod === 'crypto' ? 'opacity-40 cursor-not-allowed' : ''}`}>
              Monthly
            </button>
            <button
              onClick={() => setBillingInterval('annual')}
              className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
                billingInterval === 'annual'
                  ? 'bg-primary-500/20 text-primary-400 border border-primary-500/30'
                  : 'text-stone-500 hover:text-stone-700'
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
                        : 'border-stone-200 bg-white'
                    }`}>
                    <div className="flex items-start justify-between mb-2">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 flex-wrap">
                          <h4 className="text-sm font-semibold text-stone-900">{plan.name}</h4>
                          {/* Features inline with title */}
                          {plan.features.map(f => (
                            <span key={f.text} className="text-xs text-stone-600">
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
                          <span className="text-xl font-bold text-stone-900">
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
                        <div className="mt-2 flex flex-wrap gap-1.5">
                          <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                            Included monthly value: {formatUsdAmount(plan.monthlyBudgetUsd)}
                          </span>
                          <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                            7-day cycle: {formatUsdAmount(plan.weeklyBudgetUsd)}
                          </span>
                          <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                            5-hour cap: {formatUsdAmount(plan.fiveHourCapUsd)}
                          </span>
                          <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                            Discount: {plan.discountPercent}%
                          </span>
                          <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                            Storage: {formatStorageLimit(plan.storageLimitBytes)}
                          </span>
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

            {/* ── Payment confirmed banner ─────────────────────────── */}
            {paymentConfirmed && (
              <div className="rounded-xl bg-sage-500/10 border border-sage-500/20 p-3 mx-4">
                <div className="flex items-center gap-2">
                  <svg
                    className="w-4 h-4 text-sage-400 flex-shrink-0"
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
                  <p className="text-xs text-sage-300 font-medium">
                    Payment confirmed! Your plan has been updated.
                  </p>
                </div>
              </div>
            )}

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
                  <p className="text-xs text-amber-700">
                    Waiting for payment confirmation... Complete checkout in the browser window that
                    opened.
                  </p>
                </div>
              </div>
            )}

            {/* ── Pay with crypto toggle ────────────────────────────── */}
            <div className="flex items-center justify-between rounded-xl bg-stone-50 border border-stone-200 p-3 mx-4">
              <div>
                <p className="text-xs font-medium text-stone-900">Pay with Crypto</p>
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

            {/* ── Auto-Recharge Credits ─────────────────────────────── */}
            <div className="px-4 pt-2">
              <div className="rounded-2xl border border-stone-200 bg-white overflow-hidden">
                {/* Header row */}
                <div className="flex items-center justify-between p-3">
                  <div>
                    <p className="text-xs font-semibold text-stone-900">Auto-Recharge Credits</p>
                    <p className="text-[11px] text-stone-400 mt-0.5">
                      Automatically top up when your balance runs low
                    </p>
                  </div>
                  {arLoading ? (
                    <div className="w-10 h-5 rounded-full bg-stone-700/60 animate-pulse" />
                  ) : (
                    <button
                      onClick={handleArToggle}
                      disabled={arSaving}
                      role="switch"
                      aria-checked={arSettings?.enabled ?? false}
                      aria-label="Toggle auto-recharge"
                      className={`relative w-10 h-5 rounded-full transition-colors focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-2 focus-visible:ring-offset-stone-900 ${
                        arSaving ? 'opacity-50 cursor-not-allowed' : ''
                      } ${arSettings?.enabled ? 'bg-primary-500' : 'bg-stone-600'}`}>
                      <span
                        className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                          arSettings?.enabled ? 'translate-x-5' : 'translate-x-0'
                        }`}
                      />
                    </button>
                  )}
                </div>

                {/* Error banner */}
                {arError && (
                  <div className="mx-3 mb-2 flex items-start gap-2 rounded-lg bg-coral-500/10 border border-coral-500/20 px-2.5 py-2">
                    <svg
                      className="w-3.5 h-3.5 text-coral-400 flex-shrink-0 mt-0.5"
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
                    <p className="text-[11px] text-coral-300 leading-relaxed">{arError}</p>
                    <button
                      onClick={() => setArError(null)}
                      className="ml-auto text-coral-400 hover:text-coral-300 flex-shrink-0"
                      aria-label="Dismiss error">
                      <svg
                        className="w-3 h-3"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M6 18L18 6M6 6l12 12"
                        />
                      </svg>
                    </button>
                  </div>
                )}

                {/* Settings — only shown when enabled */}
                {!arLoading && arSettings?.enabled && (
                  <div className="border-t border-stone-200 px-3 pt-3 pb-2 space-y-3">
                    {/* Status row */}
                    <div className="flex items-center gap-3 flex-wrap">
                      {arSettings.inFlight && (
                        <span className="flex items-center gap-1 text-[10px] text-amber-700 bg-amber-50 border border-amber-200 rounded-full px-2 py-0.5">
                          <svg className="w-2.5 h-2.5 animate-spin" fill="none" viewBox="0 0 24 24">
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
                          Recharge in progress
                        </span>
                      )}
                      {arSettings.spentThisWeekUsd > 0 && (
                        <span className="text-[10px] text-stone-400">
                          ${arSettings.spentThisWeekUsd.toFixed(2)} of ${arSettings.weeklyLimitUsd}{' '}
                          used this week
                        </span>
                      )}
                      {arSettings.lastRechargeAt && (
                        <span className="text-[10px] text-stone-500">
                          Last recharged{' '}
                          {new Date(arSettings.lastRechargeAt).toLocaleDateString('en-US', {
                            month: 'short',
                            day: 'numeric',
                          })}
                        </span>
                      )}
                    </div>

                    {/* Last error from recharge attempt */}
                    {arSettings.lastError && (
                      <div className="flex items-start gap-1.5 rounded-lg bg-coral-500/10 border border-coral-500/20 px-2.5 py-2">
                        <svg
                          className="w-3 h-3 text-coral-400 flex-shrink-0 mt-0.5"
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
                        <p className="text-[10px] text-coral-300">
                          Last recharge failed: {arSettings.lastError}
                        </p>
                      </div>
                    )}

                    {/* Trigger threshold */}
                    <div>
                      <p className="text-[11px] text-stone-400 mb-1.5">
                        Recharge when balance drops below
                      </p>
                      <div className="flex gap-1.5 flex-wrap">
                        {THRESHOLD_OPTIONS.map(v => (
                          <button
                            key={v}
                            onClick={() => setArThreshold(v)}
                            className={`px-2.5 py-1 text-xs rounded-lg border transition-colors ${
                              arThreshold === v
                                ? 'bg-primary-500/20 text-primary-400 border-primary-500/40'
                                : 'bg-stone-100 text-stone-500 border-stone-200 hover:text-stone-700'
                            }`}>
                            ${v}
                          </button>
                        ))}
                      </div>
                    </div>

                    {/* Recharge amount */}
                    <div>
                      <p className="text-[11px] text-stone-400 mb-1.5">Add this amount</p>
                      <div className="flex gap-1.5 flex-wrap">
                        {RECHARGE_OPTIONS.map(v => (
                          <button
                            key={v}
                            onClick={() => setArAmount(v)}
                            className={`px-2.5 py-1 text-xs rounded-lg border transition-colors ${
                              arAmount === v
                                ? 'bg-primary-500/20 text-primary-400 border-primary-500/40'
                                : 'bg-stone-100 text-stone-500 border-stone-200 hover:text-stone-700'
                            }`}>
                            ${v}
                          </button>
                        ))}
                      </div>
                    </div>

                    {/* Weekly limit */}
                    <div>
                      <p className="text-[11px] text-stone-400 mb-1.5">Weekly spending limit</p>
                      <div className="flex gap-1.5 flex-wrap">
                        {WEEKLY_LIMIT_OPTIONS.map(v => (
                          <button
                            key={v}
                            onClick={() => setArWeeklyLimit(v)}
                            className={`px-2.5 py-1 text-xs rounded-lg border transition-colors ${
                              arWeeklyLimit === v
                                ? 'bg-primary-500/20 text-primary-400 border-primary-500/40'
                                : 'bg-stone-100 text-stone-500 border-stone-200 hover:text-stone-700'
                            }`}>
                            ${v}
                          </button>
                        ))}
                      </div>
                    </div>

                    {/* Validation hint */}
                    {arAmount <= arThreshold && (
                      <p className="text-[10px] text-amber-400">
                        Recharge amount should be greater than the trigger threshold.
                      </p>
                    )}

                    {/* Save button */}
                    {arDirty && (
                      <button
                        onClick={handleArSave}
                        disabled={arSaving || arAmount <= arThreshold}
                        className={`w-full py-1.5 text-xs font-medium rounded-lg transition-colors ${
                          arSaving || arAmount <= arThreshold
                            ? 'bg-stone-700/40 text-stone-500 cursor-not-allowed'
                            : 'bg-primary-500 hover:bg-primary-600 text-white'
                        }`}>
                        {arSaving ? 'Saving…' : 'Save Settings'}
                      </button>
                    )}
                  </div>
                )}

                {/* Payment methods */}
                <div className="border-t border-stone-200 px-3 py-2.5">
                  <div className="flex items-center justify-between mb-2">
                    <p className="text-[11px] font-medium text-stone-600">Payment Methods</p>
                    <button
                      onClick={handleAddCard}
                      className="text-[11px] text-primary-400 hover:text-primary-300 font-medium transition-colors">
                      + Add card
                    </button>
                  </div>

                  {cardsLoading ? (
                    <div className="space-y-1.5">
                      {[0, 1].map(i => (
                        <div key={i} className="h-9 rounded-lg bg-stone-700/30 animate-pulse" />
                      ))}
                    </div>
                  ) : cards.length === 0 ? (
                    <div className="flex items-center gap-2 rounded-lg bg-stone-50 border border-stone-200 p-2.5">
                      <svg
                        className="w-4 h-4 text-stone-500 flex-shrink-0"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={1.5}
                          d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z"
                        />
                      </svg>
                      <p className="text-[11px] text-stone-500">
                        No saved cards. Add one to enable auto-recharge.
                      </p>
                    </div>
                  ) : (
                    <div className="space-y-1.5">
                      {cards.map(card => {
                        const isDeleting = deletingCardId === card.id;
                        const isSettingDefault = settingDefaultId === card.id;
                        const isConfirming = confirmDeleteId === card.id;

                        return (
                          <div
                            key={card.id}
                            className="flex items-center gap-2 rounded-lg bg-stone-50 border border-stone-200 px-2.5 py-2">
                            {/* Card icon */}
                            <svg
                              className="w-4 h-4 text-stone-400 flex-shrink-0"
                              fill="none"
                              stroke="currentColor"
                              viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={1.5}
                                d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z"
                              />
                            </svg>

                            {/* Card info */}
                            <div className="flex-1 min-w-0">
                              <div className="flex items-center gap-1.5 flex-wrap">
                                <span className="text-xs text-stone-900 font-medium">
                                  {cardBrandLabel(card.brand)} ••••{card.last4}
                                </span>
                                {card.isDefault && (
                                  <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-primary-500/20 text-primary-400 border border-primary-500/30 font-medium">
                                    Default
                                  </span>
                                )}
                              </div>
                              <p className="text-[10px] text-stone-500 mt-0.5">
                                Expires {String(card.expMonth).padStart(2, '0')}/
                                {String(card.expYear).slice(-2)}
                              </p>
                            </div>

                            {/* Actions */}
                            <div className="flex items-center gap-1 flex-shrink-0">
                              {!card.isDefault && (
                                <button
                                  onClick={() => handleSetDefault(card.id)}
                                  disabled={!!settingDefaultId || !!deletingCardId}
                                  className="text-[10px] text-stone-500 hover:text-stone-700 transition-colors disabled:opacity-40 disabled:cursor-not-allowed px-1.5 py-1">
                                  {isSettingDefault ? '…' : 'Set default'}
                                </button>
                              )}

                              {isConfirming ? (
                                <div className="flex items-center gap-1">
                                  <button
                                    onClick={() => handleDeleteCard(card.id)}
                                    disabled={isDeleting}
                                    className="text-[10px] text-coral-400 hover:text-coral-300 font-medium transition-colors disabled:opacity-40 px-1.5 py-1">
                                    {isDeleting ? '…' : 'Confirm'}
                                  </button>
                                  <button
                                    onClick={() => setConfirmDeleteId(null)}
                                    className="text-[10px] text-stone-500 hover:text-stone-400 transition-colors px-1 py-1">
                                    Cancel
                                  </button>
                                </div>
                              ) : (
                                <button
                                  onClick={() => setConfirmDeleteId(card.id)}
                                  disabled={isDeleting || !!settingDefaultId}
                                  className="text-[10px] text-stone-500 hover:text-coral-400 transition-colors disabled:opacity-40 disabled:cursor-not-allowed px-1.5 py-1">
                                  Remove
                                </button>
                              )}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            </div>

            {/* ── Upgrade benefits ───────────────────────────────────── */}
            <div className="px-4 pb-4 pt-2">
              <div className="rounded-xl bg-gradient-to-br from-primary-500/10 to-sage-500/10 border border-primary-500/20 p-4">
                <h3 className="text-sm font-semibold text-stone-900 mb-2">Why upgrade?</h3>
                <ul className="space-y-1.5 text-xs text-stone-600">
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
                      Higher tiers increase your premium-usage discount and included usage every
                      cycle
                    </span>
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
                        Annual billing lowers the effective monthly price, and top-ups let you keep
                        going when usage spikes
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
