/**
 * Shared polling hook + presentational helpers for `MemoryTreeStatusPanel`.
 * Extracted so the panel component itself stays under the file-size budget —
 * behavior is unchanged, this is a pure move.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  memoryNamespaceSummaries,
  memorySyncStatusList,
  type MemorySyncStatusRow,
  memoryTreePipelineStatus,
  type MemoryTreePipelineStatus,
} from '../../utils/tauriCommands';

/** Translator function shape exposed by `useT()`. */
export type TFn = (key: string, fallback?: string) => string;

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
export function useMemoryTreeStatus(): {
  status: MemoryTreePipelineStatus | null;
  /** Stored-document grand total from the mandatory namespaces surface. */
  storedItems: number | null;
  integrations: MemorySyncStatusRow[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [status, setStatus] = useState<MemoryTreePipelineStatus | null>(null);
  const [storedItems, setStoredItems] = useState<number | null>(null);
  const [integrations, setIntegrations] = useState<MemorySyncStatusRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);
  const statusRef = useRef<MemoryTreePipelineStatus | null>(null);
  statusRef.current = status;

  const fetchOnce = useCallback(async () => {
    console.debug('[ui-flow][memory-tree-status] fetchOnce: entry');
    // Stored-document total rides the same poll: it is the sync-verification
    // number, from a different store than every tree tile (#5932 field
    // finding — a user watched 100 items land while the tree figure sat at 2).
    void memoryNamespaceSummaries()
      .then(r => {
        if (!cancelledRef.current) setStoredItems(r.total_documents);
      })
      .catch(() => {
        if (!cancelledRef.current) setStoredItems(null);
      });
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

  return { status, storedItems, integrations, loading, error, refresh: fetchOnce };
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
export function formatRelativeMs(ms: number, t: TFn, neverLabel: string): string {
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
export function formatBytes(n: number): string {
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
export function statusDotClass(kind: MemoryTreePipelineStatus['status']): string {
  switch (kind) {
    case 'running':
      return 'bg-sage-400';
    case 'syncing':
      return 'bg-sage-500 animate-pulse';
    case 'paused':
      return 'bg-content-faint';
    case 'error':
      return 'bg-coral-500';
    case 'degraded':
      // Amber: the pipeline is running but recall/structure is reduced.
      return 'bg-amber-500';
    case 'idle':
    default:
      return 'bg-content-faint';
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
type IntegrationHealth = 'active' | 'stale';

/** Map the wire `freshness` enum to the two-state UI classification. */
export function classifyIntegration(
  freshness: MemorySyncStatusRow['freshness']
): IntegrationHealth {
  return freshness === 'active' ? 'active' : 'stale';
}

/**
 * Built-in glyph for each known provider key from `memory_sync_status_list`.
 * Source: `MemorySyncStatus.provider` in `src/openhuman/memory/sync/sync_status/types.rs`
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
export function IntegrationHealthStrip({
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
            const dot = health === 'active' ? 'bg-sage-400' : 'bg-content-faint';
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
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-surface px-2 py-0.5 text-[11px] font-medium text-content-secondary ring-1 ring-line-strong">
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
