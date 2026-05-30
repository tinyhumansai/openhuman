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
  memoryTreePipelineStatus,
  type MemoryTreePipelineStatus,
  memoryTreeSetEnabled,
} from '../../utils/tauriCommands';

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
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [status, setStatus] = useState<MemoryTreePipelineStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);
  const statusRef = useRef<MemoryTreePipelineStatus | null>(null);
  statusRef.current = status;

  const fetchOnce = useCallback(async () => {
    console.debug('[ui-flow][memory-tree-status] fetchOnce: entry');
    try {
      const next = await memoryTreePipelineStatus();
      if (cancelledRef.current) return;
      setStatus(next);
      setError(null);
      console.debug(
        '[ui-flow][memory-tree-status] fetchOnce: ok status=%s total=%d',
        next.status,
        next.total_chunks
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

  return { status, loading, error, refresh: fetchOnce };
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
    case 'idle':
    default:
      return 'bg-stone-400 dark:bg-neutral-500';
  }
}

/**
 * Memory Tree status panel — render the four-tile dashboard plus the
 * auto-sync toggle. Designed to mount above `<MemorySources>` in
 * `MemoryWorkspace` so it surfaces in both the Intelligence page and
 * Settings → Memory data without extra wiring.
 */
export function MemoryTreeStatusPanel({ onToast }: MemoryTreeStatusPanelProps) {
  const { t } = useT();
  const { status, loading, error, refresh } = useMemoryTreeStatus();
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
      case 'idle':
      default:
        return t('memoryTree.status.statusIdle');
    }
  })();

  const checked = !(status?.is_paused ?? false);

  const tileClass =
    'rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800';
  const labelClass =
    'text-[11px] uppercase tracking-wide text-stone-500 dark:text-neutral-400 mb-1';
  const valueClass = 'text-xl font-semibold text-stone-900 dark:text-neutral-100';
  const skeletonClass = 'h-7 w-16 rounded bg-stone-200 dark:bg-neutral-800 animate-pulse';

  return (
    <div className="space-y-3" data-testid="memory-tree-status-panel">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('memoryTree.status.title')}
        </h2>
      </div>

      {error && !loading ? (
        <div
          role="alert"
          className="flex items-center justify-between gap-3 rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-sm text-coral-700 dark:text-coral-300"
          data-testid="memory-tree-status-error">
          <span>{t('memoryTree.status.fetchError')}</span>
          <button
            type="button"
            onClick={() => {
              void refresh();
            }}
            className="rounded-md border border-coral-300 dark:border-coral-500/40 bg-white dark:bg-neutral-900 px-2 py-1 text-xs font-medium text-coral-700 dark:text-coral-300 hover:bg-coral-50 dark:hover:bg-coral-500/20">
            {t('memoryTree.status.retry')}
          </button>
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
                <div className="mt-0.5 text-[11px] text-stone-500 dark:text-neutral-400">
                  {status.reason}
                </div>
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

      {/* Auto-sync toggle row — markup mirrors AIPanel's inline ToggleRow */}
      <div
        className="flex items-center justify-between gap-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2"
        data-testid="memory-tree-status-toggle-row">
        <div className="min-w-0">
          <div className="text-sm font-medium text-stone-900 dark:text-neutral-100">
            {t('memoryTree.status.autoSyncLabel')}
          </div>
          <div className="text-xs text-stone-500 dark:text-neutral-400">
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
            checked ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-700'
          }`}>
          <span
            aria-hidden
            className={`inline-block h-4 w-4 transform rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${
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
