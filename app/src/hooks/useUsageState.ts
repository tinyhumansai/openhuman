import { useCallback, useEffect, useState } from 'react';

import { billingApi } from '../services/api/billingApi';
import { creditsApi, type TeamUsage } from '../services/api/creditsApi';
import type { CurrentPlanData, PlanTier } from '../types/api';

export interface UsageState {
  teamUsage: TeamUsage | null;
  currentPlan: CurrentPlanData | null;
  currentTier: PlanTier;
  isFreeTier: boolean;
  usagePct10h: number;
  usagePct7d: number;
  isNearLimit: boolean;
  isAtLimit: boolean;
  isRateLimited: boolean;
  isBudgetExhausted: boolean;
  isLoading: boolean;
  refresh: () => void;
}

const CACHE_TTL_MS = 60_000;

let _cache: {
  data: { teamUsage: TeamUsage; currentPlan: CurrentPlanData };
  fetchedAt: number;
} | null = null;

async function fetchUsageData(): Promise<{ teamUsage: TeamUsage; currentPlan: CurrentPlanData }> {
  if (_cache && Date.now() - _cache.fetchedAt < CACHE_TTL_MS) {
    return _cache.data;
  }
  const [teamUsage, currentPlan] = await Promise.all([
    creditsApi.getTeamUsage(),
    billingApi.getCurrentPlan(),
  ]);
  _cache = { data: { teamUsage, currentPlan }, fetchedAt: Date.now() };
  return _cache.data;
}

export function useUsageState(): UsageState {
  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [currentPlan, setCurrentPlan] = useState<CurrentPlanData | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [fetchCount, setFetchCount] = useState(0);

  const refresh = useCallback(() => {
    _cache = null;
    setFetchCount(c => c + 1);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setIsLoading(true);
    fetchUsageData()
      .then(data => {
        if (cancelled) return;
        setTeamUsage(data.teamUsage);
        setCurrentPlan(data.currentPlan);
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
    teamUsage && teamUsage.fiveHourCapUsd > 0
      ? Math.min(1, teamUsage.cycleLimit5hr / teamUsage.fiveHourCapUsd)
      : 0;

  const usagePct7d =
    teamUsage && teamUsage.cycleBudgetUsd > 0
      ? Math.min(1, (teamUsage.cycleBudgetUsd - teamUsage.remainingUsd) / teamUsage.cycleBudgetUsd)
      : 0;

  const isBudgetExhausted = teamUsage ? teamUsage.remainingUsd <= 0 : false;

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
    currentTier,
    isFreeTier,
    usagePct10h,
    usagePct7d,
    isNearLimit,
    isAtLimit,
    isRateLimited,
    isBudgetExhausted,
    isLoading,
    refresh,
  };
}
