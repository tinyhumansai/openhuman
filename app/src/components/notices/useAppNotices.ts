/**
 * Every notice the app raises, as one list.
 *
 * The problem this solves is fragmentation: the same state was surfaced twice
 * in two different chromes. "Memory has stopped growing" existed both as a
 * classified `UserActionableError` (panel) and as a full-width
 * `MemoryEmbeddingBudgetBanner` pushed above every route, and the usage-limit
 * upsell had a third. A banner that displaces page content is the loudest
 * possible treatment for something the user often cannot act on right now, and
 * three of them could stack.
 *
 * So the sources are merged here and rendered once, by {@link NoticeCenter}.
 * Adding a source means adding a block to this hook — not another
 * shell-mounted component.
 *
 * Ordering is severity-first (error, then warning, then info) and stable
 * within a severity, so the most serious notice is what the collapsed FAB
 * summarises.
 */
import { useCallback, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { formatResetTime } from '../../features/conversations/utils/format';
import { useEmbeddingBudgetState } from '../../hooks/useEmbeddingBudgetState';
import { useUsageState } from '../../hooks/useUsageState';
import { useT } from '../../lib/i18n/I18nContext';
import { applyOpenRouterFreeModels } from '../../services/api/openrouterFreeModels';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { selectActiveUserErrors } from '../../store/userErrorsSelectors';
import { dismissUserError, resolveUserError } from '../../store/userErrorsSlice';
import type { UserActionableError, UserErrorAction } from '../../types/userError';
import { PRICING_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import { dismissBanner, shouldShowBanner } from '../upsell/upsellDismissState';
import { useMemoryQuarantinePoll } from './useMemoryQuarantinePoll';

export type NoticeSeverity = 'error' | 'warning' | 'info';

export interface AppNotice {
  /** Stable across re-renders — also the React key and the dismissal key. */
  id: string;
  severity: NoticeSeverity;
  title: string;
  body: string;
  /** Optional footnote (source · time · recurrence). */
  meta?: string;
  /** Primary next step. Omitted when there is nothing to click. */
  actionLabel?: string;
  onAction?: () => void;
  /**
   * An alternative remediation, when a notice genuinely has two.
   *
   * Exists for the exhausted-budget case, whose banner offered both "top up"
   * and "switch to OpenRouter's free models" — the second is the only fix that
   * costs nothing, so collapsing to one CTA would have quietly removed the
   * option a user without a card actually needs.
   */
  secondaryActionLabel?: string;
  onSecondaryAction?: () => void;
  /** Omitted when the notice must not be silenced. */
  onDismiss?: () => void;
}

/** Deep-link target for each classified-error action. `dismiss` has no route. */
const ACTION_ROUTE: Record<Exclude<UserErrorAction, 'dismiss'>, string> = {
  open_billing: '/settings/billing',
  open_provider_settings: '/settings/llm',
  // #5324: both memory-embedding remediations (local Ollama, BYO key) live on
  // this one screen, so a single CTA covers them without the user needing to
  // know which one applies.
  open_embeddings_settings: '/connections?tab=embeddings',
  // Opening this screen also restarts the integration health poll, so it is
  // both the explanation and the retry.
  open_connections: '/connections?tab=skills',
  // openhuman#5820: after a corrupt-store quarantine the rebuilt tree is
  // empty; the per-source Sync and All In controls that repopulate it live on
  // Brain's Sources tab (the Sync tab only shows status and history).
  open_memory_sync: '/brain?tab=sources',
};

const ACTION_LABEL_KEY: Record<Exclude<UserErrorAction, 'dismiss'>, string> = {
  open_billing: 'userErrors.action.openBilling',
  open_provider_settings: 'userErrors.action.openProviderSettings',
  open_embeddings_settings: 'userErrors.action.openEmbeddingsSettings',
  open_connections: 'userErrors.action.openConnections',
  open_memory_sync: 'userErrors.action.openMemorySync',
};

const SEVERITY_RANK: Record<NoticeSeverity, number> = { error: 0, warning: 1, info: 2 };

/**
 * How long a dismissed near-limit warning stays quiet.
 *
 * Carried over from the chat banner this notice replaced, which persisted its
 * dismissal via `upsellDismissState`. The other derived notices use the
 * in-memory set below, but usage creeps up over days — re-nagging on every
 * restart is what the 24h cooldown was there to stop.
 */
const NEAR_LIMIT_COOLDOWN_MS = 24 * 60 * 60 * 1000;
const NEAR_LIMIT_BANNER_ID = 'conversations-warning';

/** Highest severity present, or `null` for an empty list. */
export function peakSeverity(notices: readonly AppNotice[]): NoticeSeverity | null {
  return notices.reduce<NoticeSeverity | null>(
    (peak, notice) =>
      peak === null || SEVERITY_RANK[notice.severity] < SEVERITY_RANK[peak]
        ? notice.severity
        : peak,
    null
  );
}

// Wall-clock read for resolve/dismiss timestamps, at module scope so the
// component body never calls an impure function during render
// (react-hooks/purity). Only reached from event handlers.
const nowMs = (): number => Date.now();

export function useAppNotices(): AppNotice[] {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const active = useAppSelector(selectActiveUserErrors);
  const { level: budgetLevel, pct: budgetPct } = useEmbeddingBudgetState();
  // openhuman#5820: durable, app-wide replay of a corrupt-store quarantine.
  useMemoryQuarantinePoll();
  const {
    teamUsage,
    isLoading: usageLoading,
    isAtLimit,
    isNearLimit,
    isFreeTier,
    usagePct,
    shouldShowBudgetCompletedMessage,
  } = useUsageState();

  // Per-session, per-key dismissal for the derived (non-store) notices.
  //
  // Keyed by notice id, and the embedding-budget id carries its level, so
  // silencing the 75% warning does NOT also silence the 90% escalation — that
  // would put the user back in the silent failure this whole surface exists to
  // prevent. Deliberately not persisted: a warning that survives a restart it
  // no longer applies to is worse than one shown twice.
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(() => new Set());
  const dismiss = useCallback((id: string) => {
    setDismissed(prev => new Set(prev).add(id));
  }, []);

  // The OpenRouter switch can fail, and the banner this replaced showed that
  // failure inline. Surfacing it as its own notice keeps that feedback rather
  // than letting the click look like it silently did nothing.
  const [openRouterFailed, setOpenRouterFailed] = useState(false);
  const useOpenRouterFree = useCallback(() => {
    setOpenRouterFailed(false);
    void applyOpenRouterFreeModels().catch((error: unknown) => {
      console.warn('[notices] applyOpenRouterFreeModels failed', error);
      setOpenRouterFailed(true);
    });
  }, []);

  const runErrorAction = useCallback(
    (entry: UserActionableError) => {
      if (entry.action !== 'dismiss') navigate(ACTION_ROUTE[entry.action]);
      dispatch(resolveUserError({ id: entry.id, at: nowMs() }));
    },
    [dispatch, navigate]
  );

  return useMemo(() => {
    const notices: AppNotice[] = [];

    // ── Classified runtime errors (#3931) ──────────────────────────────
    for (const entry of active) {
      notices.push({
        id: entry.id,
        severity: entry.severity,
        title: t(entry.titleKey),
        // `detail` is the source's own user-facing text (see the field's docs);
        // it says what actually failed, which the translated body cannot.
        body: entry.detail ? `${t(entry.bodyKey)}\n\n${entry.detail}` : t(entry.bodyKey),
        meta: [
          t(`userErrors.scope.${entry.scope}`, entry.scope),
          new Date(entry.lastSeenAt).toLocaleTimeString(),
          entry.count > 1 ? `×${entry.count}` : null,
        ]
          .filter(Boolean)
          .join(' · '),
        ...(entry.action !== 'dismiss'
          ? {
              actionLabel: t(ACTION_LABEL_KEY[entry.action]),
              onAction: () => runErrorAction(entry),
            }
          : {}),
        onDismiss: () => dispatch(dismissUserError({ id: entry.id })),
      });
    }

    // ── Memory embedding budget (#5324) ────────────────────────────────
    // The id carries the level so an escalation is a *new* notice rather than
    // a mutation of a dismissed one.
    const budgetId = `memory-embedding-budget:${budgetLevel}`;
    if (budgetLevel !== 'none' && !dismissed.has(budgetId)) {
      const exhausted = budgetLevel === 'exhausted';
      notices.push({
        id: budgetId,
        // Exhausted is not a warning: memory has already stopped growing.
        severity: exhausted ? 'error' : 'warning',
        title: exhausted ? t('memoryBudget.exhaustedTitle') : t('memoryBudget.approachingTitle'),
        body: exhausted
          ? t('memoryBudget.exhaustedMessage')
          : t('memoryBudget.approachingMessage').replace('{pct}', String(budgetPct)),
        actionLabel: t('memoryBudget.cta'),
        onAction: () => navigate(ACTION_ROUTE.open_embeddings_settings),
        // Only the early warning can be silenced; the escalations cannot.
        ...(budgetLevel === 'warn' ? { onDismiss: () => dismiss(budgetId) } : {}),
      });
    }

    // ── Plan usage limit ───────────────────────────────────────────────
    if (!usageLoading && teamUsage) {
      const upsellId = isAtLimit ? 'usage:at-limit' : 'usage:near-limit';
      const show = isAtLimit
        ? true
        : isNearLimit &&
          isFreeTier &&
          shouldShowBanner(NEAR_LIMIT_BANNER_ID, NEAR_LIMIT_COOLDOWN_MS);
      if (show && !dismissed.has(upsellId)) {
        notices.push({
          id: upsellId,
          severity: isAtLimit ? 'error' : 'warning',
          title: isAtLimit ? t('upsell.global.limitTitle') : t('upsell.global.nearLimitTitle'),
          body: isAtLimit
            ? t('upsell.global.limitMessage')
            : t('upsell.global.nearLimitMessage').replace(
                '{pct}',
                String(Math.round(usagePct * 100))
              ),
          actionLabel: t('chat.upgrade'),
          onAction: () => void openUrl(PRICING_URL),
          // At the limit the app is already gated, so silencing it would hide
          // the only explanation of why nothing works.
          ...(isAtLimit
            ? {}
            : {
                onDismiss: () => {
                  dismissBanner(NEAR_LIMIT_BANNER_ID);
                  dismiss(upsellId);
                },
              }),
        });
      }
    }

    // ── Included cycle budget spent ────────────────────────────────────
    // Rendered in four places before this (both Conversations layouts, the
    // new-window hero, Home) off one account-level flag. Same state, four
    // copies of the copy — so it is one notice now.
    if (teamUsage && shouldShowBudgetCompletedMessage) {
      const resets =
        teamUsage.cycleEndsAt != null
          ? ` ${t('chat.resets')} ${formatResetTime(teamUsage.cycleEndsAt)}.`
          : '';
      notices.push({
        id: 'usage:cycle-budget-spent',
        severity: 'error',
        title: t('home.usageExhaustedTitle'),
        body:
          teamUsage.cycleBudgetUsd > 0
            ? `${t('chat.weeklyLimitHit')}${resets} ${t('chat.topUpToContinue')}`
            : t('chat.budgetComplete'),
        actionLabel: t('chat.topUp'),
        onAction: () => void openUrl(PRICING_URL),
        // The free-tier escape hatch the banner carried. Kept because it is the
        // only remediation that does not require a payment method.
        secondaryActionLabel: t('openrouterFree.cta'),
        onSecondaryAction: useOpenRouterFree,
      });
    }

    if (openRouterFailed) {
      notices.push({
        id: 'openrouter-free:failed',
        severity: 'warning',
        title: t('openrouterFree.cta'),
        body: t('openrouterFree.error'),
        onDismiss: () => setOpenRouterFailed(false),
      });
    }

    return notices.sort((a, b) => SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity]);
  }, [
    active,
    budgetLevel,
    budgetPct,
    dismiss,
    dismissed,
    dispatch,
    isAtLimit,
    isFreeTier,
    isNearLimit,
    navigate,
    openRouterFailed,
    runErrorAction,
    useOpenRouterFree,
    t,
    shouldShowBudgetCompletedMessage,
    teamUsage,
    usageLoading,
    usagePct,
  ]);
}
