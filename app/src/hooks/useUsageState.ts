import { useCallback, useEffect, useState } from 'react';

import { billingApi } from '../services/api/billingApi';
import { creditsApi, type TeamUsage } from '../services/api/creditsApi';
import { CoreRpcError } from '../services/coreRpcClient';
import type { CurrentPlanData, PlanTier } from '../types/api';
import { subscribeUsageRefresh } from './usageRefresh';

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
  shouldShowBudgetCompletedMessage: boolean;
  isLoading: boolean;
  refresh: () => void;
}

const CACHE_TTL_MS = 60_000;

let _cache: {
  data: { teamUsage: TeamUsage; currentPlan: CurrentPlanData };
  fetchedAt: number;
} | null = null;

const USAGE_UNAVAILABLE = Symbol('usage-unavailable');

async function fetchUsageData(): Promise<{
  teamUsage: TeamUsage | null;
  currentPlan: CurrentPlanData | null;
} | null> {
  if (_cache && Date.now() - _cache.fetchedAt < CACHE_TTL_MS) {
    return _cache.data;
  }
  // Wrap each leg so a single failing call (e.g. /teams returning 401 after
  // session expiry) cannot reject the Promise.all microtask before the
  // sibling resolves — that race let the unhandled rejection leak to the
  // window's unhandledrejection trap and onward to Sentry (#1472).
  const [teamUsage, currentPlan] = await Promise.all([
    creditsApi.getTeamUsage().catch(err => {
      if (err instanceof CoreRpcError && err.kind === 'auth_expired') {
        throw err;
      }
      return USAGE_UNAVAILABLE;
    }),
    billingApi.getCurrentPlan().catch(err => {
      if (err instanceof CoreRpcError && err.kind === 'auth_expired') {
        throw err;
      }
      return USAGE_UNAVAILABLE;
    }),
  ]);
  const data = {
    teamUsage: teamUsage === USAGE_UNAVAILABLE ? null : (teamUsage as TeamUsage),
    currentPlan: currentPlan === USAGE_UNAVAILABLE ? null : (currentPlan as CurrentPlanData),
  };
  if (data.teamUsage && data.currentPlan) {
    _cache = {
      data: { teamUsage: data.teamUsage, currentPlan: data.currentPlan },
      fetchedAt: Date.now(),
    };
  }
  return data;
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

  useEffect(() => subscribeUsageRefresh(refresh), [refresh]);

  useEffect(() => {
    let cancelled = false;
    setIsLoading(true);
    fetchUsageData()
      .then(data => {
        if (cancelled || !data) return;
        setTeamUsage(data.teamUsage);
        setCurrentPlan(data.currentPlan);
      })
      .catch((err: unknown) => {
        // CoreRpcError(kind=auth_expired) is the documented signal that the
        // session has been revoked — coreRpcClient already dispatched the
        // global reauth event, so swallow here instead of letting it leak
        // to window.unhandledrejection -> Sentry (#1472).
        if (err instanceof CoreRpcError && err.kind === 'auth_expired') return;
        // Other failures: usage unavailable — silently ignore.
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

  // Some users have no included recurring budget at all. They still need the
  // completed-budget warning in chat even though they are not in an exhausted
  // paid cycle.
  const shouldShowBudgetCompletedMessage = teamUsage
    ? isBudgetExhausted || (teamUsage.cycleBudgetUsd <= 0.01 && teamUsage.remainingUsd <= 0.01)
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
