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
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import {
  memorySyncStatusList,
  type MemorySyncStatusRow,
  memoryTreePipelineStatus,
  type MemoryTreePipelineStatus,
  memoryTreeSetEnabled,
} from '../../utils/tauriCommands';
import Button from '../ui/Button';

/** Translator function shape exposed by `useT()`. */
type TFn = (key: string, fallback?: string) => string;

/**
 * Adaptive polling cadence — match the existing memory ingestion
 * panel so the two surfaces feel like one.
 */
const FAST_POLL_MS = 1500;
const DEFAULT_POLL_MS = 4000;

/**
 * Public hook so unit tests (and any future caller) can subscribe to the
 * pipeline-status stream without re-implementing the polling dance.
 */
function useMemoryTreeStatus(): {
  status: MemoryTreePipelineStatus | null;
  integrations: MemorySyncStatusRow[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [status, setStatus] = useState<MemoryTreePipelineStatus | null>(null);
  const [integrations, setIntegrations] = useState<MemorySyncStatusRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);
  const statusRef = useRef<MemoryTreePipelineStatus | null>(null);
  statusRef.current = status;

  const fetchOnce = useCallback(async () => {
    console.debug('[ui-flow][memory-tree-status] fetchOnce: entry');
    try {
      // Fetch pipeline + per-integration health in parallel so the strip
      // and the tiles share a single 1.5s / 4s adaptive tick (#2763).
      const [next, rows] = await Promise.all([
        memoryTreePipelineStatus(),
        memorySyncStatusList().catch(err => {
          // Per-integration list is best-effort: surface an empty strip
          // rather than wiping the panel when only the secondary endpoint
          // fails. Pipeline failure still flips the panel-wide error.
          console.warn(
            '[ui-flow][memory-tree-status] memorySyncStatusList failed: %s',
            err instanceof Error ? err.message : String(err)
          );
          return [] as MemorySyncStatusRow[];
        }),
      ]);
      if (cancelledRef.current) return;
      setStatus(next);
      setIntegrations(rows);
      setError(null);
      console.debug(
        '[ui-flow][memory-tree-status] fetchOnce: ok status=%s total=%d integrations=%d',
        next.status,
        next.total_chunks,
        rows.length
      );
    } catch (err) {
      if (cancelledRef.current) return;
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ui-flow][memory-tree-status] fetchOnce: error %s', message);
      setError(message);
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    cancelledRef.current = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      await fetchOnce();
      if (cancelledRef.current) return;
      const live = statusRef.current;
      const fast = live?.is_syncing || (live?.pipeline_jobs?.running ?? 0) > 0;
      timer = setTimeout(tick, fast ? FAST_POLL_MS : DEFAULT_POLL_MS);
    };

    void tick();

    return () => {
      cancelledRef.current = true;
      if (timer) clearTimeout(timer);
    };
  }, [fetchOnce]);

  return { status, integrations, loading, error, refresh: fetchOnce };
}

interface MemoryTreeStatusPanelProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

/**
 * Format a millisecond timestamp as a coarse "5 min ago" style label.
 * Returns the localized `Never` placeholder when `ms` is 0/falsy.
 *
 * Intentionally light — no dayjs dependency, no plural rules. Buckets
 * (just-now / seconds / minutes / hours / days) are enough for the status
 * tile; the precise timestamp is one level deeper in the workspace UI.
 *
 * Strings flow through `t()` from `useT()` so the panel localizes
 * cleanly. `{count}` placeholders are substituted client-side because
 * `t()` does not interpolate (see `I18nContext.tsx`).
 */
function formatRelativeMs(ms: number, t: TFn, neverLabel: string): string {
  if (!ms || ms <= 0) return neverLabel;
  const diffMs = Date.now() - ms;
  if (diffMs < 0) return neverLabel; // clock skew safety
  const sec = Math.floor(diffMs / 1000);
  if (sec < 30) return t('memoryTree.status.justNow');
  if (sec < 60) return t('memoryTree.status.secondsAgo').replace('{count}', String(sec));
  const min = Math.floor(sec / 60);
  if (min < 60) {
    if (min === 1) return t('memoryTree.status.minuteAgo');
    return t('memoryTree.status.minutesAgo').replace('{count}', String(min));
  }
  const hr = Math.floor(min / 60);
  if (hr < 24) {
    if (hr === 1) return t('memoryTree.status.hourAgo');
    return t('memoryTree.status.hoursAgo').replace('{count}', String(hr));
  }
  const day = Math.floor(hr / 24);
  if (day === 1) return t('memoryTree.status.dayAgo');
  return t('memoryTree.status.daysAgo').replace('{count}', String(day));
}

/**
 * Format a raw byte count as KiB / MiB / GiB — sized to the order of
 * magnitude. Negative / zero ⇒ `0 B`.
 */
function formatBytes(n: number): string {
  if (!n || n <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  // 1 decimal place once we're past bytes, integer for plain bytes.
  return `${i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

/** Map the wire status to a dot color token + animation flag. */
function statusDotClass(kind: MemoryTreePipelineStatus['status']): string {
  switch (kind) {
    case 'running':
      return 'bg-sage-400';
    case 'syncing':
      return 'bg-sage-500 animate-pulse';
    case 'paused':
      return 'bg-stone-400 dark:bg-neutral-500';
    case 'error':
      return 'bg-coral-500';
    case 'degraded':
      // Amber: the pipeline is running but recall/structure is reduced.
      return 'bg-amber-500';
    case 'idle':
    default:
      return 'bg-stone-400 dark:bg-neutral-500';
  }
}

/**
 * UI health classification for a single provider row in the integration
 * health strip (#2763). The wire shape's three-state `freshness` collapses
 * to two states here — `Active` (currently producing chunks) vs `Stale`
 * (anything older). An `Error` state is intentionally NOT derived from the
 * current data; per-provider failure attribution needs new core work and
 * is filed as a follow-up to issue #2763.
 */
export type IntegrationHealth = 'active' | 'stale';

/** Map the wire `freshness` enum to the two-state UI classification. */
export function classifyIntegration(
  freshness: MemorySyncStatusRow['freshness']
): IntegrationHealth {
  return freshness === 'active' ? 'active' : 'stale';
}

/**
 * Built-in glyph for each known provider key from `memory_sync_status_list`.
 * Source: `MemorySyncStatus.provider` in `src/openhuman/memory_sync/sync_status/types.rs`
 * — that file's doc comment enumerates the providers ("slack", "gmail",
 * "discord", "telegram", "whatsapp", "notion", "meeting_notes",
 * "drive_docs", etc.). Anything not in this map falls back to a generic
 * plug glyph so unknown providers still render cleanly.
 *
 * Kept inline (rather than re-using `SOURCE_KIND_ICONS` from
 * `memorySourcesService`) because that map is keyed by `SourceKind`
 * (`composio` / `folder` / `github_repo` / …) — a different taxonomy.
 */
const PROVIDER_ICONS: Record<string, string> = {
  slack: '💬',
  gmail: '📧',
  discord: '🎮',
  telegram: '✈️',
  whatsapp: '🟢',
  notion: '📝',
  meeting_notes: '🎙️',
  drive_docs: '📄',
  github: '🐙',
};

/** Look up a provider glyph; fall back to a generic plug for unknowns. */
export function providerIconChar(provider: string): string {
  return PROVIDER_ICONS[provider] ?? '🔌';
}

/**
 * Per-integration health strip (#2763). Rendered between the four pipeline
 * tiles and the auto-sync toggle inside `MemoryTreeStatusPanel`. Consumes
 * the `integrations` slice returned by `useMemoryTreeStatus` — no
 * additional fetch, no second timer.
 */
function IntegrationHealthStrip({
  integrations,
  loading,
  t,
}: {
  integrations: MemorySyncStatusRow[];
  loading: boolean;
  t: TFn;
}) {
  return (
    <div className="space-y-2" data-testid="memory-tree-integrations">
      <div className="text-[11px] uppercase tracking-wide text-content-muted">
        {t('memoryTree.status.integrationsTitle')}
      </div>
      {loading && integrations.length === 0 ? (
        // First-mount: suppress "no integrations" copy until the initial poll
        // resolves, otherwise the strip falsely implies nothing is connected
        // before data arrives (CodeRabbit feedback on #2763).
        <div
          data-testid="memory-tree-integrations-skeleton"
          className="h-9 animate-pulse rounded-lg bg-surface-strong"
        />
      ) : integrations.length === 0 ? (
        <div
          data-testid="memory-tree-integrations-empty"
          className="rounded-lg border border-dashed border-line px-3 py-2 text-xs text-content-muted">
          {t('memoryTree.status.integrationsEmpty')}
        </div>
      ) : (
        <ul
          className="max-h-48 space-y-1 overflow-y-auto rounded-lg border border-line bg-surface-muted/40 dark:bg-surface-muted/30 p-2"
          aria-label={t('memoryTree.status.integrationsTitle')}>
          {integrations.map(row => {
            const health = classifyIntegration(row.freshness);
            const healthLabel =
              health === 'active'
                ? t('memoryTree.status.integrationActive')
                : t('memoryTree.status.integrationStale');
            const dot = health === 'active' ? 'bg-sage-400' : 'bg-stone-400 dark:bg-neutral-500';
            return (
              <li
                key={row.provider}
                data-testid={`memory-tree-integration-row-${row.provider}`}
                className="flex items-center justify-between gap-2 rounded-md px-2 py-1.5 hover:bg-surface-subtle/60 dark:hover:bg-surface-muted/60">
                <div className="flex min-w-0 items-center gap-2">
                  <span aria-hidden className="text-base leading-none">
                    {providerIconChar(row.provider)}
                  </span>
                  <span className="truncate text-sm font-medium text-content">{row.provider}</span>
                </div>
                <div className="flex shrink-0 items-center gap-3 text-xs text-content-muted">
                  <span>
                    {t('memoryTree.status.integrationChunks').replace(
                      '{count}',
                      new Intl.NumberFormat().format(row.chunks_synced)
                    )}
                  </span>
                  <span>
                    {formatRelativeMs(row.last_chunk_at_ms ?? 0, t, t('memoryTree.status.never'))}
                  </span>
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-surface px-2 py-0.5 text-[11px] font-medium text-content-secondary ring-1 ring-stone-200 dark:ring-neutral-700">
                    <span aria-hidden className={`inline-block h-1.5 w-1.5 rounded-full ${dot}`} />
                    {healthLabel}
                  </span>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/**
 * Memory Tree status panel — render the four-tile dashboard plus the
 * auto-sync toggle. Designed to mount above `<MemorySources>` in
 * `MemoryWorkspace` so it surfaces in both the Intelligence page and
 * Settings → Memory data without extra wiring.
 */
export function MemoryTreeStatusPanel({ onToast }: MemoryTreeStatusPanelProps) {
  const { t } = useT();
  const { status, integrations, loading, error, refresh } = useMemoryTreeStatus();
  const [toggleBusy, setToggleBusy] = useState(false);

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

  const statusKind = status?.status ?? 'idle';
  const statusLabel: string = (() => {
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

  // #002 (FR-004): the single first blocking cause, rendered verbatim with a
  // localized remediation. Prefer the explicit `first_blocking_cause`; fall
  // back to the active degradation cause so older payload shapes still surface
  // something actionable.
  const blockingCause = status?.first_blocking_cause ?? status?.degraded?.cause ?? null;
  const degraded = status?.degraded;

  const checked = !(status?.is_paused ?? false);

  const tileClass =
    'rounded-xl border border-line bg-surface-muted p-3 transition-colors hover:bg-surface-hover';
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
        <div className={tileClass}>
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
            </>
          )}
        </div>

        {/* Last-sync tile */}
        <div className={tileClass}>
          <div className={labelClass}>{t('memoryTree.status.lastSyncTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-tree-last-sync">
              {formatRelativeMs(status.last_sync_ms, t, t('memoryTree.status.never'))}
            </div>
          )}
        </div>

        {/* Total chunks tile */}
        <div className={tileClass}>
          <div className={labelClass}>{t('memoryTree.status.totalChunksTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-tree-total-chunks">
              {new Intl.NumberFormat().format(status.total_chunks)}
            </div>
          )}
        </div>

        {/* Wiki size tile */}
        <div className={tileClass}>
          <div className={labelClass}>{t('memoryTree.status.wikiSizeTile')}</div>
          {loading || !status ? (
            <div className={skeletonClass} />
          ) : (
            <div className={valueClass} data-testid="memory-tree-wiki-size">
              {formatBytes(status.wiki_size_bytes)}
            </div>
          )}
        </div>
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
        <button
          type="button"
          role="switch"
          aria-label={t('memoryTree.status.autoSyncLabel')}
          aria-checked={checked}
          disabled={toggleBusy || loading || !status}
          onClick={() => {
            void handleToggle();
          }}
          data-testid="memory-tree-status-toggle"
          className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-60 ${
            checked ? 'bg-primary-500' : 'bg-surface-strong'
          }`}>
          <span
            aria-hidden
            className={`inline-block h-4 w-4 transform rounded-full bg-surface shadow transition-transform ${
              checked ? 'translate-x-4' : 'translate-x-0.5'
            }`}
          />
        </button>
      </div>
    </div>
  );
}

// Re-export the hook so unit tests can opt into the polling subscription
// directly without re-implementing it.
export { useMemoryTreeStatus };
