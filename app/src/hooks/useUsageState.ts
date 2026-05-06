import { useCallback, useEffect, useState } from 'react';

import { billingApi } from '../services/api/billingApi';
import { type CreditBalance, creditsApi, type TeamUsage } from '../services/api/creditsApi';
import type { CurrentPlanData, PlanTier } from '../types/api';
import { subscribeUsageRefresh } from './usageRefresh';

export interface UsageState {
  teamUsage: TeamUsage | null;
  currentPlan: CurrentPlanData | null;
  creditBalance: CreditBalance | null;
  currentTier: PlanTier;
  isFreeTier: boolean;
  usagePct10h: number;
  usagePct7d: number;
  isNearLimit: boolean;
  isAtLimit: boolean;
  isRateLimited: boolean;
  isBudgetExhausted: boolean;
  shouldShowBudgetCompletedMessage: boolean;
  isLoading: boolean;
  refresh: () => void;
}

const CACHE_TTL_MS = 60_000;

let _cache: {
  data: { teamUsage: TeamUsage; currentPlan: CurrentPlanData; creditBalance: CreditBalance };
  fetchedAt: number;
} | null = null;

async function fetchUsageData(): Promise<{
  teamUsage: TeamUsage;
  currentPlan: CurrentPlanData;
  creditBalance: CreditBalance;
}> {
  if (_cache && Date.now() - _cache.fetchedAt < CACHE_TTL_MS) {
    return _cache.data;
  }
  const [teamUsage, currentPlan, creditBalance] = await Promise.all([
    creditsApi.getTeamUsage(),
    billingApi.getCurrentPlan(),
    creditsApi
      .getBalance()
      .catch((): CreditBalance => ({ promotionBalanceUsd: 0, teamTopupUsd: 0 })),
  ]);
  _cache = { data: { teamUsage, currentPlan, creditBalance }, fetchedAt: Date.now() };
  return _cache.data;
}

export function useUsageState(): UsageState {
  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [currentPlan, setCurrentPlan] = useState<CurrentPlanData | null>(null);
  const [creditBalance, setCreditBalance] = useState<CreditBalance | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [fetchCount, setFetchCount] = useState(0);

  const refresh = useCallback(() => {
    _cache = null;
    setFetchCount(c => c + 1);
  }, []);

  useEffect(() => subscribeUsageRefresh(refresh), [refresh]);

  useEffect(() => {
    let cancelled = false;
    setIsLoading(true);
    fetchUsageData()
      .then(data => {
        if (cancelled) return;
        setTeamUsage(data.teamUsage);
        setCurrentPlan(data.currentPlan);
        setCreditBalance(data.creditBalance);
      })
      .catch(() => {
        // Usage unavailable — silently ignore
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fetchCount]);

  const currentTier: PlanTier = currentPlan?.plan ?? 'FREE';
  const isFreeTier = currentTier === 'FREE';

  const usagePct10h =
    teamUsage && teamUsage.fiveHourCapUsd > 0.01
      ? Math.min(1, teamUsage.cycleLimit5hr / teamUsage.fiveHourCapUsd)
      : 0;

  const usagePct7d =
    teamUsage && teamUsage.cycleBudgetUsd > 0.01
      ? Math.min(1, (teamUsage.cycleBudgetUsd - teamUsage.remainingUsd) / teamUsage.cycleBudgetUsd)
      : 0;

  const isBudgetExhausted = teamUsage
    ? teamUsage.cycleBudgetUsd > 0.01 && teamUsage.remainingUsd <= 0.01
    : false;

  // Top-up and promotional credits should suppress the budget-completed banner
  // even when the included recurring budget is exhausted.
  const hasAvailableCredits = creditBalance
    ? creditBalance.teamTopupUsd + creditBalance.promotionBalanceUsd > 0.01
    : false;

  // Show the banner only when ALL credit sources are exhausted: included budget,
  // top-up balance, and promotional credits.
  const shouldShowBudgetCompletedMessage = teamUsage
    ? !hasAvailableCredits &&
      (isBudgetExhausted || (teamUsage.cycleBudgetUsd <= 0.01 && teamUsage.remainingUsd <= 0.01))
    : false;

  const isRateLimited =
    teamUsage !== null &&
    !teamUsage.bypassCycleLimit &&
    teamUsage.fiveHourCapUsd > 0 &&
    teamUsage.cycleLimit5hr >= teamUsage.fiveHourCapUsd;

  const isAtLimit = isBudgetExhausted || isRateLimited;

  const isNearLimit = !isAtLimit && teamUsage !== null && (usagePct10h >= 0.8 || usagePct7d >= 0.8);

  return {
    teamUsage,
    currentPlan,
    creditBalance,
    currentTier,
    isFreeTier,
    usagePct10h,
    usagePct7d,
    isNearLimit,
    isAtLimit,
    isRateLimited,
    isBudgetExhausted,
    shouldShowBudgetCompletedMessage,
    isLoading,
    refresh,
  };
}
