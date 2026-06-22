import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useCoreState } from '../providers/CoreStateProvider';
import {
  type AISettings,
  ALL_WORKLOADS,
  CHAT_WORKLOADS,
  loadAISettings,
} from '../services/api/aiSettingsApi';
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
  usagePct: number;
  isNearLimit: boolean;
  isAtLimit: boolean;
  isBudgetExhausted: boolean;
  shouldShowBudgetCompletedMessage: boolean;
  /**
   * True when every chat workload (reasoning/agentic/coding) is routed to a
   * non-openhuman provider (a user-configured cloud provider or local Ollama).
   * Used to suppress the OpenHuman-included-budget banner / modal: when the
   * user has explicitly bypassed the hosted backend for chat, the included
   * budget cycle no longer gates them. See #2040 and #2041.
   */
  isFullyRoutedAway: boolean;
  isLoading: boolean;
  refresh: () => void;
}

const logBillingGate = debug('openhuman:billing:gate');

const CACHE_TTL_MS = 60_000;

let _cache: {
  data: { teamUsage: TeamUsage; currentPlan: CurrentPlanData; aiSettings: AISettings | null };
  fetchedAt: number;
} | null = null;

const USAGE_UNAVAILABLE = Symbol('usage-unavailable');

function workloadsRoutedAway(aiSettings: AISettings, workloads: readonly string[]): boolean {
  return workloads.every(w => {
    const ref = aiSettings.routing[w as keyof AISettings['routing']];
    return ref !== undefined && ref.kind !== 'openhuman';
  });
}

async function fetchUsageData(): Promise<{
  teamUsage: TeamUsage | null;
  currentPlan: CurrentPlanData | null;
  aiSettings: AISettings | null;
} | null> {
  // Read routing first. If every workload is explicitly assigned to a local
  // or user-supplied cloud provider, this session should not phone home to
  // OpenHuman's billing/usage APIs at all (#2020). Missing/failed AI settings
  // stay conservative and fall through to the existing billing path.
  const aiSettings = await loadAISettings().catch(err => {
    if (err instanceof CoreRpcError && err.kind === 'auth_expired') {
      throw err;
    }
    return USAGE_UNAVAILABLE;
  });
  if (
    aiSettings !== USAGE_UNAVAILABLE &&
    workloadsRoutedAway(aiSettings as AISettings, ALL_WORKLOADS)
  ) {
    return { teamUsage: null, currentPlan: null, aiSettings: aiSettings as AISettings };
  }
  if (_cache && Date.now() - _cache.fetchedAt < CACHE_TTL_MS) {
    return {
      ..._cache.data,
      aiSettings: aiSettings === USAGE_UNAVAILABLE ? null : (aiSettings as AISettings),
    };
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
    aiSettings: aiSettings === USAGE_UNAVAILABLE ? null : (aiSettings as AISettings),
  };
  if (data.teamUsage && data.currentPlan) {
    _cache = {
      data: {
        teamUsage: data.teamUsage,
        currentPlan: data.currentPlan,
        aiSettings: data.aiSettings,
      },
      fetchedAt: Date.now(),
    };
  }
  return data;
}

/**
 * @param activeChatRole the chat-mode tier the caller is gating on — `chat` for
 * Quick mode (default), `reasoning` for Reasoning mode. The credits bypass is
 * checked against this tier so the prompt reflects the mode the user selected.
 */
export function useUsageState(activeChatRole: 'chat' | 'reasoning' = 'chat'): UsageState {
  const { snapshot } = useCoreState();
  const isAuthenticated = snapshot.auth.isAuthenticated;
  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [currentPlan, setCurrentPlan] = useState<CurrentPlanData | null>(null);
  const [aiSettings, setAiSettings] = useState<AISettings | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [fetchCount, setFetchCount] = useState(0);

  const refresh = useCallback(() => {
    _cache = null;
    setFetchCount(c => c + 1);
  }, []);

  useEffect(() => subscribeUsageRefresh(refresh), [refresh]);

  useEffect(() => {
    // Gate on auth BEFORE dispatching: `team_get_usage` / `billing_get_current_plan`
    // require a backend session, so polling them while signed out (pre-login, or
    // after a `SessionExpired` clear) is a guaranteed 401 — the Sentry
    // TAURI-RUST-8WY (`/teams/me/usage`) / 8WZ (`/payments/stripe/currentPlan`)
    // flood (#3297). When unauthenticated, skip the fetch and drop any stale view
    // instead of round-tripping to a doomed call. The core-side
    // `require_live_session_token` precheck covers the expired-but-still-stored
    // window (token present so `isAuthenticated` is still true) without a network
    // call; this gate covers the absent-token windows.
    if (!isAuthenticated) {
      _cache = null;
      setTeamUsage(null);
      setCurrentPlan(null);
      setAiSettings(null);
      setIsLoading(false);
      return;
    }
    let cancelled = false;
    setIsLoading(true);
    fetchUsageData()
      .then(data => {
        if (cancelled || !data) return;
        setTeamUsage(data.teamUsage);
        setCurrentPlan(data.currentPlan);
        setAiSettings(data.aiSettings);
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
  }, [fetchCount, isAuthenticated]);

  const currentTier: PlanTier = currentPlan?.plan ?? 'FREE';
  const isFreeTier = currentTier === 'FREE';

  const usagePct =
    teamUsage && teamUsage.cycleBudgetUsd > 0.01
      ? Math.max(
          0,
          Math.min(
            1,
            (teamUsage.cycleBudgetUsd - teamUsage.remainingUsd) / teamUsage.cycleBudgetUsd
          )
        )
      : 0;

  // When every chat workload routes to a user-supplied provider (cloud or
  // local Ollama), the OpenHuman included-budget cycle does not gate the
  // user. Conservative on missing aiSettings (treat as still using
  // openhuman) so we never silently disable the gate after a transient
  // fetch failure (#2040, #2041).
  //
  // #3767: prefer the authoritative, core-side `creditsBypass` decision for the
  // selected chat-mode tier (Quick → `chat`, Reasoning → `reasoning`) — true when
  // that tier runs on a usable non-managed provider the user funds themselves —
  // and OR it with the existing routing-string heuristic. This closes the gap
  // where the selected mode runs on a BYO provider but the raw routing strings
  // still read as managed, so the buy-credits prompt stayed up.
  const creditsBypassForMode = aiSettings?.creditsBypass?.[activeChatRole] === true;
  const isFullyRoutedAway = aiSettings
    ? creditsBypassForMode || workloadsRoutedAway(aiSettings, CHAT_WORKLOADS)
    : false;

  const rawBudgetExhausted = teamUsage
    ? teamUsage.cycleBudgetUsd > 0.01 && teamUsage.remainingUsd <= 0.01
    : false;

  // Only show the completed-budget warning for an actually exhausted
  // recurring budget. Free plans with no recurring budget should not look like
  // they have exhausted a paid/included cycle (#2129).
  const rawShouldShowBudgetCompletedMessage = rawBudgetExhausted;

  const isBudgetExhausted = !isFullyRoutedAway && rawBudgetExhausted;
  const shouldShowBudgetCompletedMessage =
    !isFullyRoutedAway && rawShouldShowBudgetCompletedMessage;

  const isAtLimit = isBudgetExhausted;

  // Mirror the isAtLimit guard: when every chat workload is routed away from
  // OpenHuman the included-budget cycle does not gate the user, so the
  // near-limit warning is equally irrelevant (#3097 — top-up banner shown
  // despite custom provider).
  const isNearLimit = !isAtLimit && !isFullyRoutedAway && teamUsage !== null && usagePct >= 0.8;

  // #3767: verbose gate-decision diagnostics — which branch (gated vs bypassed)
  // and why. Keyed on the inputs so it only fires when the decision changes.
  useEffect(() => {
    if (!teamUsage && !aiSettings) return;
    logBillingGate(
      `[billing][gate] mode=${activeChatRole} budgetExhausted=${rawBudgetExhausted} ` +
        `creditsBypass=${creditsBypassForMode} ` +
        `fullyRoutedAway=${isFullyRoutedAway} -> ${isAtLimit ? 'GATED' : 'bypassed'}`
    );
  }, [
    activeChatRole,
    creditsBypassForMode,
    rawBudgetExhausted,
    isFullyRoutedAway,
    isAtLimit,
    teamUsage,
  ]);

  return {
    teamUsage,
    currentPlan,
    currentTier,
    isFreeTier,
    usagePct,
    isNearLimit,
    isAtLimit,
    isBudgetExhausted,
    shouldShowBudgetCompletedMessage,
    isFullyRoutedAway,
    isLoading,
    refresh,
  };
}
