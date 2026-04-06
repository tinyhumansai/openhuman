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
import AutoRechargeSection from './billing/AutoRechargeSection';
import InferenceBudget from './billing/InferenceBudget';
import PayAsYouGoCard from './billing/PayAsYouGoCard';
import SubscriptionPlans from './billing/SubscriptionPlans';
import { buildPlanId, formatStorageLimit, formatUsdAmount, getPlanMeta } from './billingHelpers';

const log = createDebug('openhuman:billing-panel');

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
      const refreshed = await creditsApi.getAutoRecharge();
      setArSettings(refreshed);
    } catch {
      setArError('Failed to remove card.');
    } finally {
      setDeletingCardId(null);
    }
  };

  const handleAddCard = async () => {
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
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      setIsPurchasing(false);
      setPurchasingTier(null);
      setPaymentConfirmed(true);

      try {
        const plan = await billingApi.getCurrentPlan();
        log('[payment-success] plan=%s active=%s', plan.plan, plan.hasActiveSubscription);
        setCurrentPlan(plan);
      } catch (e) {
        console.error('Failed to fetch current plan after payment', e);
      }
      await refresh();

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

  const handleBalanceRefresh = useCallback(async () => {
    try {
      const balance = await creditsApi.getBalance();
      setCreditBalance(balance);
    } catch (err) {
      log('[balance-refresh] failed: %O', err);
    }
  }, []);

  // ── JSX ─────────────────────────────────────────────────────────────
  return (
    <div>
      <SettingsHeader
        title={teamName ? `Billing — ${teamName}` : 'Billing & Subscription'}
        showBackButton={true}
        onBack={navigateBack}
      />

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

            {/* ── Pay as You Go ── PROMOTED TO TOP ─────────────────── */}
            <PayAsYouGoCard
              creditBalance={creditBalance}
              isLoadingCredits={isLoadingCredits}
              isToppingUp={isToppingUp}
              onTopUp={handleTopUp}
              onBalanceRefresh={handleBalanceRefresh}
            />
          </div>

          {/* ── Divider ──────────────────────────────────────────── */}
          <div className="flex items-center gap-3 px-4 py-2 max-w-md mx-auto">
            <div className="flex-1 h-px bg-stone-200" />
            <span className="text-xs text-stone-400 whitespace-nowrap">
              Or subscribe for included usage + discounts
            </span>
            <div className="flex-1 h-px bg-stone-200" />
          </div>

          <div className="max-w-md mx-auto">
            {/* ── Subscription Plans ──────────────────────────────── */}
            <SubscriptionPlans
              currentTier={currentTier}
              billingInterval={billingInterval}
              setBillingInterval={setBillingInterval}
              paymentMethod={paymentMethod}
              setPaymentMethod={setPaymentMethod}
              isPurchasing={isPurchasing}
              purchasingTier={purchasingTier}
              paymentConfirmed={paymentConfirmed}
              onUpgrade={handleUpgrade}
            />

            {/* ── Inference Budget ────────────────────────────────── */}
            <div className="px-4 pt-2">
              <InferenceBudget teamUsage={teamUsage} isLoadingCredits={isLoadingCredits} />
            </div>

            {/* ── Auto-Recharge + Payment Methods ────────────────── */}
            <AutoRechargeSection
              arSettings={arSettings}
              arLoading={arLoading}
              arError={arError}
              arSaving={arSaving}
              arThreshold={arThreshold}
              arAmount={arAmount}
              arWeeklyLimit={arWeeklyLimit}
              arDirty={arDirty}
              setArThreshold={setArThreshold}
              setArAmount={setArAmount}
              setArWeeklyLimit={setArWeeklyLimit}
              setArError={setArError}
              onArToggle={handleArToggle}
              onArSave={handleArSave}
              cards={cards}
              cardsLoading={cardsLoading}
              confirmDeleteId={confirmDeleteId}
              deletingCardId={deletingCardId}
              settingDefaultId={settingDefaultId}
              setConfirmDeleteId={setConfirmDeleteId}
              onSetDefault={handleSetDefault}
              onDeleteCard={handleDeleteCard}
              onAddCard={handleAddCard}
            />

            {/* ── Why upgrade? ───────────────────────────────────── */}
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
