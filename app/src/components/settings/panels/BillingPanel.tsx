import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import PillTabBar from '../../../components/PillTabBar';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { billingApi } from '../../../services/api/billingApi';
import {
  type AutoRechargeSettings,
  type CreditBalance,
  creditsApi,
  type CreditTransaction,
  type SavedCard,
} from '../../../services/api/creditsApi';
import type { CurrentPlanData, PlanTier } from '../../../types/api';
import { openUrl } from '../../../utils/openUrl';
import PageBackButton from '../components/PageBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import BillingHistoryTab from './billing/BillingHistoryTab';
import BillingPaymentsTab from './billing/BillingPaymentsTab';
import BillingPlansTab from './billing/BillingPlansTab';
import { buildPlanId } from './billingHelpers';

const log = createDebug('openhuman:billing-panel');

type BillingTab = 'overview' | 'plans' | 'payments' | 'history';

// ── Component ───────────────────────────────────────────────────────────
const BillingPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, teams, refresh } = useCoreState();
  const user = snapshot.currentUser;
  const sessionToken = snapshot.sessionToken;

  // Active team context
  const activeTeamId = user?.activeTeamId;
  const activeTeam = teams.find(t => t.team._id === activeTeamId);
  const teamName = activeTeam?.team.name;

  // Credits & usage state
  const [currentPlan, setCurrentPlan] = useState<CurrentPlanData | null>(null);
  const [creditBalance, setCreditBalance] = useState<CreditBalance | null>(null);
  const [transactions, setTransactions] = useState<CreditTransaction[]>([]);
  const [isLoadingCredits, setIsLoadingCredits] = useState(false);
  const [isToppingUp, setIsToppingUp] = useState(false);
  const [selectedTab, setSelectedTab] = useState<BillingTab>('plans');

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
  // Fetch current plan, credits balance, and team usage once auth is available.
  useEffect(() => {
    if (!sessionToken) {
      log('[load] skipped: no session token yet');
      setCurrentPlan(null);
      setCreditBalance(null);
      setIsLoadingCredits(false);
      return;
    }

    let cancelled = false;
    setIsLoadingCredits(true);
    log('[load] fetching billing state tokenPresent=%s activeTeamId=%s', true, activeTeamId);
    Promise.allSettled([
      billingApi.getCurrentPlan(),
      creditsApi.getBalance(),
      creditsApi.getTransactions(5, 0),
    ])
      .then(([planResult, balanceResult, transactionsResult]) => {
        if (planResult.status === 'fulfilled') {
          const plan = planResult.value;
          log(
            '[load] plan=%s active=%s weeklyBudget=%s',
            plan.plan,
            plan.hasActiveSubscription,
            plan.weeklyBudgetUsd
          );
          if (!cancelled) {
            setCurrentPlan(plan);
          }
        } else {
          log('[load] getCurrentPlan failed: %O', planResult.reason);
        }
        if (balanceResult.status === 'fulfilled') {
          log(
            '[load] balance promotion=%s teamTopup=%s',
            balanceResult.value.promotionBalanceUsd,
            balanceResult.value.teamTopupUsd
          );
          if (!cancelled) {
            setCreditBalance(balanceResult.value);
          }
        } else {
          log('[load] getBalance failed: %O', balanceResult.reason);
        }
        if (transactionsResult.status === 'fulfilled') {
          if (!cancelled) {
            setTransactions(transactionsResult.value.transactions);
          }
        } else {
          log('[load] getTransactions failed: %O', transactionsResult.reason);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoadingCredits(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [sessionToken, activeTeamId]);

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
      setArError('Add a payment card on Stripe before enabling auto-recharge.');
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
        const balance = await creditsApi.getBalance();
        log(
          '[payment-success] refreshed balance promotion=%s teamTopup=%s',
          balance.promotionBalanceUsd,
          balance.teamTopupUsd
        );
        setCreditBalance(balance);
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
      log(
        '[balance-refresh] promotion=%s teamTopup=%s',
        balance.promotionBalanceUsd,
        balance.teamTopupUsd
      );
      setCreditBalance(balance);
    } catch (err) {
      log('[balance-refresh] failed: %O', err);
    }
  }, []);

  const transactionRows = transactions.slice(0, 4);

  // ── JSX ─────────────────────────────────────────────────────────────
  return (
    <div className="overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-5 px-4 py-6 sm:px-6 sm:py-8">
        <header className="space-y-5">
          <PageBackButton
            label={
              teamName ? `Back to ${breadcrumbs.at(-1)?.label ?? 'settings'}` : 'Back to settings'
            }
            onClick={navigateBack}
          />
          <PillTabBar
            items={[
              { label: 'Plans', value: 'plans' },
              { label: 'Top ups & Credits', value: 'payments' },
              { label: 'History', value: 'history' },
            ]}
            selected={selectedTab}
            onChange={setSelectedTab}
            activeClassName="border-primary-600 bg-primary-600 text-white"
            inactiveClassName="border-stone-200 bg-white text-stone-600 hover:bg-stone-50"
            containerClassName="flex gap-2 overflow-x-auto pb-1 scrollbar-hide"
          />
        </header>

        {selectedTab === 'plans' && (
          <BillingPlansTab
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
        )}

        {selectedTab === 'payments' && (
          <BillingPaymentsTab
            arAmount={arAmount}
            arDirty={arDirty}
            arError={arError}
            arLoading={arLoading}
            arSaving={arSaving}
            arSettings={arSettings}
            arThreshold={arThreshold}
            arWeeklyLimit={arWeeklyLimit}
            cards={cards}
            cardsLoading={cardsLoading}
            confirmDeleteId={confirmDeleteId}
            creditBalance={creditBalance}
            deletingCardId={deletingCardId}
            isLoadingCredits={isLoadingCredits}
            isToppingUp={isToppingUp}
            onAddCard={handleAddCard}
            onArSave={handleArSave}
            onArToggle={handleArToggle}
            onBalanceRefresh={handleBalanceRefresh}
            onDeleteCard={handleDeleteCard}
            onSetDefault={handleSetDefault}
            onTopUp={handleTopUp}
            setArAmount={setArAmount}
            setArThreshold={setArThreshold}
            setArWeeklyLimit={setArWeeklyLimit}
            setConfirmDeleteId={setConfirmDeleteId}
            settingDefaultId={settingDefaultId}
          />
        )}

        {selectedTab === 'history' && (
          <BillingHistoryTab
            hasActive={hasActive}
            onManageSubscription={handleManageSubscription}
            transactionRows={transactionRows}
          />
        )}
      </div>
    </div>
  );
};

export default BillingPanel;
