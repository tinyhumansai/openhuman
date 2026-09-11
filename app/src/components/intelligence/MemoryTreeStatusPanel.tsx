/**
 * Memory Tree status panel — 4 stat tiles + on/off toggle.
 *
 * Replaces the temporary `useConsciousItems`-driven pill in
 * `Intelligence.tsx`; addresses issue #1856 Part 1.
 *
 * The toggle writes `config.scheduler_gate.mode = "off"` via the
 * `memory_tree_set_enabled` RPC and relies on the scheduler-gate
 * hot-reload to pause all LLM-bound background work cooperatively.
 * It does NOT pause the 20-min Composio fetch loop yet (#1856 Part 2
 * follow-up).
 *
 * Polling cadence mirrors `useMemoryIngestionStatus`: 1.5s while
 * syncing/active jobs, 4s otherwise — the same heuristic we use
 * elsewhere so the dashboard feels lively without thrashing the
 * core.
 *
 * Layout & color conventions copied verbatim from
 * `MemoryStatsBar.tsx` (tiles) and the inline `ToggleRow` in
 * `settings/panels/AIPanel.tsx` (switch markup).
 */
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { reportMemoryPipelineFailure, reportMemoryQuarantine } from '../../lib/userErrors/report';
import { useAppDispatch } from '../../store/hooks';
import type { ToastNotification } from '../../types/intelligence';
import { memoryTreeRetryFailed, memoryTreeSetEnabled } from '../../utils/tauriCommands';
import { trackAnalyticsEvent } from '../analytics';
import { Card } from '../ui';
import Button from '../ui/Button';
import Switch from '../ui/Switch';
import {
  classifyIntegration,
  formatBytes,
  formatRelativeMs,
  IntegrationHealthStrip,
  providerIconChar,
  statusDotClass,
  useMemoryTreeStatus,
} from './memoryTreeStatusHelpers';

export { classifyIntegration, providerIconChar };

interface MemoryTreeStatusPanelProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

/**
 * Memory Tree status panel — render the four-tile dashboard plus the
 * auto-sync toggle. Designed to mount above `<MemorySources>` in
 * `MemoryWorkspace` so it surfaces in both the Intelligence page and
 * Settings → Memory data without extra wiring.
 */
export function MemoryTreeStatusPanel({ onToast }: MemoryTreeStatusPanelProps) {
  const { t } = useT();
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const { status, storedItems, integrations, loading, error, refresh } = useMemoryTreeStatus();
  const [toggleBusy, setToggleBusy] = useState(false);
  const [retryBusy, setRetryBusy] = useState(false);

  // #002 (FR-004): the single first blocking cause. Prefer the explicit
  // `first_blocking_cause`; fall back to the active degradation cause so older
  // payload shapes still surface something actionable. Derived ONCE here so the
  // escalation, the status label, the CTA, and the banner all key off the same
  // cause — a payload that carries only `degraded.cause` (and no
  // `first_blocking_cause`) must not render the banner one way while the
  // escalation and budget label read a different, empty cause.
  const blockingCause = status?.first_blocking_cause ?? status?.degraded?.cause ?? null;

  // #5324: this panel was the ONLY place a budget-exhausted memory pipeline
  // was ever surfaced, so users who never opened it experienced weeks of
  // silently broken memory. Escalate the typed cause into the shell-mounted
  // UserErrorCenter, which stays visible across routes and after the panel
  // unmounts. The store dedupes on descriptor id, so polling re-reports bump
  // the recurrence count rather than stacking entries.
  const blockingCauseCode = blockingCause?.code ?? null;
  useEffect(() => {
    reportMemoryPipelineFailure(dispatch, blockingCauseCode);
  }, [dispatch, blockingCauseCode]);
  // openhuman#5820: this panel polls faster than the shell, so a re-sync
  // retires the quarantine notice as soon as the first chunk lands.
  const quarantine = status?.quarantine ?? null;
  const quarantineKey = quarantine
    ? `${quarantine.quarantined_at_ms}:${quarantine.resynced}`
    : null;
  useEffect(() => {
    reportMemoryQuarantine(dispatch, quarantine);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the fields that matter
  }, [dispatch, quarantineKey]);

  const handleToggle = useCallback(async () => {
    if (!status || toggleBusy) return;
    const nextEnabled = status.is_paused; // currently paused ⇒ enable
    console.debug('[ui-flow][memory-tree-status] toggle: entry next_enabled=%s', nextEnabled);
    setToggleBusy(true);
    try {
      await memoryTreeSetEnabled(nextEnabled);
      await refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ui-flow][memory-tree-status] toggle: error %s', message);
      onToast?.({ type: 'error', title: t('memoryTree.status.toggleFailed'), message });
    } finally {
      setToggleBusy(false);
    }
  }, [status, toggleBusy, refresh, onToast, t]);

  /**
   * Requeue every terminally-failed job.
   *
   * An unrecoverable failure (auth, budget, dimension mismatch) is terminal by
   * design — the worker never retries it — so a batch that failed under a
   * since-fixed config stays parked forever and pins this panel on `error`.
   * The `memory_tree_retry_failed` RPC existed for exactly this, but had no
   * caller anywhere in the app, leaving the user with a permanent error state
   * and no way to clear it.
   */
  const handleRetryFailed = useCallback(async () => {
    if (retryBusy) {
      console.debug('[ui-flow][memory-tree-status] retryFailed: skipped busy=true');
      return;
    }
    console.debug('[ui-flow][memory-tree-status] retryFailed: entry');
    setRetryBusy(true);
    console.debug('[ui-flow][memory-tree-status] retryFailed: busy=true rpc:start');
    try {
      const { requeued } = await memoryTreeRetryFailed();
      console.debug('[ui-flow][memory-tree-status] retryFailed: rpc:ok requeued=%d', requeued);
      // Record the successful domain outcome (not just the click). Privacy-safe:
      // a non-identifying count only, no ids or user text.
      trackAnalyticsEvent('memory_tree_retry_succeeded', { count: requeued });
      onToast?.({
        type: 'success',
        title: t('memoryTree.status.retryFailedDone'),
        message: t('memoryTree.status.retryFailedCount').replace('{count}', String(requeued)),
      });
      await refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ui-flow][memory-tree-status] retryFailed: error %s', message);
      onToast?.({ type: 'error', title: t('memoryTree.status.retryFailedError'), message });
    } finally {
      console.debug('[ui-flow][memory-tree-status] retryFailed: busy=false exit');
      setRetryBusy(false);
    }
  }, [retryBusy, refresh, onToast, t]);

  const statusKind = status?.status ?? 'idle';
  // #5324: "Error — 936 unrecoverable failures need action" told the user
  // nothing they could act on. When the blocking cause is a spent embedding
  // budget, name that state exactly. Derived here rather than as a new wire
  // status so older clients keep deserialising the payload unchanged and the
  // existing `status` precedence rules stay untouched.
  //
  // Scoped to the states the budget actually explains. `first_blocking_cause`
  // reports the most recent failed job even when the user has since paused the
  // tree themselves, so without this guard a manually-paused tree carrying an
  // old budget failure would be relabelled and hide the real reason it stopped.
  const isBudgetExhausted =
    blockingCause?.code === 'budget_exhausted' &&
    (statusKind === 'error' || statusKind === 'degraded');
  const statusLabel: string = (() => {
    if (isBudgetExhausted) return t('memoryTree.status.statusBudgetExhausted');
    switch (statusKind) {
      case 'running':
        return t('memoryTree.status.statusRunning');
      case 'paused':
        return t('memoryTree.status.statusPaused');
      case 'syncing':
        return t('memoryTree.status.statusSyncing');
      case 'error':
        return t('memoryTree.status.statusError');
      case 'degraded':
        return t('memoryTree.status.statusDegraded');
      case 'idle':
      default:
        return t('memoryTree.status.statusIdle');
    }
  })();

  // `blockingCause` (derived above) is rendered verbatim in the banner below
  // with a localized remediation.
  const degraded = status?.degraded;

  // Parked failures are the one panel state the user can act on directly, and
  // the affordance is keyed off the counter rather than off the blocking-cause
  // banner: a failure the pipeline has already worked past no longer surfaces a
  // remediation, but its rows still sit in `failed` and still need clearing.
  const failedJobs = status?.pipeline_jobs.failed ?? 0;

  const checked = !(status?.is_paused ?? false);

  const labelClass = 'text-[11px] uppercase tracking-wide text-content-muted mb-1';
  const valueClass = 'text-xl font-semibold text-content';
  const skeletonClass = 'h-7 w-16 rounded bg-surface-strong animate-pulse';

  return (
    <div className="space-y-3" data-testid="memory-tree-status-panel">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-content">{t('memoryTree.status.title')}</h2>
      </div>

      {error && !loading ? (
        <div
          role="alert"
          className="flex items-center justify-between gap-3 rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-sm text-coral-700 dark:text-coral-300"
          data-testid="memory-tree-status-error">
          <span>{t('memoryTree.status.fetchError')}</span>
          <Button
            variant="secondary"
            tone="danger"
            size="xs"
            onClick={() => {
              void refresh();
            }}>
            {t('memoryTree.status.retry')}
          </Button>
        </div>
      ) : null}

      {/* #002 (FR-004): actionable first-blocking-cause banner. Shown when the
          core reports a typed cause — names the problem + the fix instead of a
          generic "error". Degraded badges below distinguish recall vs structure. */}
      {!loading && blockingCause ? (
        <div
          className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-sm text-amber-800 dark:text-amber-200"
          data-testid="memory-tree-blocking-cause">
          <div className="font-medium" data-testid="memory-tree-blocking-cause-remediation">
            {t(blockingCause.remediation_key, t('memory.health.remediation.unknown'))}
          </div>
          {/* #5324: the remediation text names the fix ("set up local Ollama
              embeddings or add your own key") but left the user to find that
              screen themselves. One click, no embeddings knowledge required. */}
          {isBudgetExhausted ? (
            <div className="mt-2">
              <Button
                variant="secondary"
                size="xs"
                data-testid="memory-tree-budget-cta"
                analyticsId="memory-tree-budget-configure-embeddings"
                onClick={() => {
                  navigate('/connections?tab=embeddings');
                }}>
                {t('userErrors.action.openEmbeddingsSettings')}
              </Button>
            </div>
          ) : null}
          {degraded?.semantic_recall || degraded?.structure ? (
            <div className="mt-1 flex flex-wrap gap-1.5" data-testid="memory-tree-degraded-badges">
              {degraded?.semantic_recall ? (
                <span
                  className="inline-flex items-center rounded-full bg-amber-100 dark:bg-amber-500/20 px-2 py-0.5 text-[11px] font-medium text-amber-800 dark:text-amber-200"
                  data-testid="memory-tree-badge-recall">
                  {t('memoryTree.status.degradedRecall')}
                </span>
              ) : null}
              {degraded?.structure ? (
                <span
                  className="inline-flex items-center rounded-full bg-amber-100 dark:bg-amber-500/20 px-2 py-0.5 text-[11px] font-medium text-amber-800 dark:text-amber-200"
                  data-testid="memory-tree-badge-structure">
                  {t('memoryTree.status.degradedStructure')}
                </span>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3" data-testid="memory-tree-status-tiles">
        {/* Status tile ── color-coded pill */}
        <Card
          divided={false}
          className="bg-surface-muted p-3 transition-colors hover:bg-surface-hover">
          <div className={labelClass}>{t('memoryTree.status.statusTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <>
              <div className="flex items-center gap-2">
                <span
                  aria-hidden
                  className={`inline-block h-2 w-2 rounded-full ${statusDotClass(statusKind)}`}
                />
                <span className={valueClass} data-testid="memory-tree-status-label">
                  {statusLabel}
                </span>
              </div>
              {status.reason ? (
                <div className="mt-0.5 text-[11px] text-content-muted">{status.reason}</div>
              ) : null}
              {failedJobs > 0 ? (
                <div className="mt-2">
                  <Button
                    variant="secondary"
                    size="xs"
                    disabled={retryBusy}
                    data-testid="memory-tree-retry-failed"
                    analyticsId="memory-tree-retry-failed-jobs"
                    onClick={() => {
                      void handleRetryFailed();
                    }}>
                    {retryBusy
                      ? t('memoryTree.status.retryFailedBusy')
                      : t('memoryTree.status.retryFailed')}
                  </Button>
                </div>
              ) : null}
            </>
          )}
        </Card>

        {/* Last-sync tile */}
        <Card
          divided={false}
          className="bg-surface-muted p-3 transition-colors hover:bg-surface-hover">
          <div className={labelClass}>{t('memoryTree.status.lastSyncTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-tree-last-sync">
              {formatRelativeMs(status.last_sync_ms, t, t('memoryTree.status.never'))}
            </div>
          )}
        </Card>

        {/* Stored items tile — the document/search-index store, the number a
            user checks after a sync. Distinct from the summary-tree leaves
            tile beside it, which counts a different, much smaller store. */}
        <Card
          divided={false}
          className="bg-surface-muted p-3 transition-colors hover:bg-surface-hover">
          <div className={labelClass}>{t('memoryTree.status.storedItemsTile')}</div>
          {storedItems === null ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-stored-items">
              {new Intl.NumberFormat().format(storedItems)}
            </div>
          )}
        </Card>

        {/* Total chunks tile */}
        <Card
          divided={false}
          className="bg-surface-muted p-3 transition-colors hover:bg-surface-hover">
          <div className={labelClass}>{t('memoryTree.status.totalChunksTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-tree-total-chunks">
              {new Intl.NumberFormat().format(status.total_chunks)}
            </div>
          )}
        </Card>

        {/* Wiki size tile */}
        <Card
          divided={false}
          className="bg-surface-muted p-3 transition-colors hover:bg-surface-hover">
          <div className={labelClass}>{t('memoryTree.status.wikiSizeTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-tree-wiki-size">
              {formatBytes(status.wiki_size_bytes)}
            </div>
          )}
        </Card>
      </div>

      {/* #002 (FR-010 / US5): extraction coverage. Only meaningful once chunks
          exist; near-0% with chunks present means the wiki is built but has no
          structure (the extraction model is failing). */}
      {!loading && status && status.total_chunks > 0 && status.extraction_coverage != null ? (
        <div className="text-xs text-content-muted" data-testid="memory-tree-extraction-coverage">
          {t('memoryTree.status.extractionCoverage').replace(
            '{pct}',
            String(Math.round((status.extraction_coverage ?? 0) * 100))
          )}
        </div>
      ) : null}

      <IntegrationHealthStrip integrations={integrations} loading={loading} t={t} />

      {/* Auto-sync toggle row — markup mirrors AIPanel's inline ToggleRow */}
      <div
        className="flex items-center justify-between gap-3 rounded-lg border border-line bg-surface px-3 py-2"
        data-testid="memory-tree-status-toggle-row">
        <div className="min-w-0">
          <div className="text-sm font-medium text-content">
            {t('memoryTree.status.autoSyncLabel')}
          </div>
          <div className="text-xs text-content-muted">
            {t('memoryTree.status.autoSyncDescription')}
          </div>
        </div>
        <Switch
          id="memory-tree-status-toggle"
          aria-label={t('memoryTree.status.autoSyncLabel')}
          checked={checked}
          disabled={toggleBusy || loading || !status}
          onCheckedChange={() => {
            void handleToggle();
          }}
          data-testid="memory-tree-status-toggle"
        />
      </div>
    </div>
  );
}
