/**
 * Unified memory sources panel.
 *
 * Single source of truth for **what feeds memory**: folders, GitHub
 * repos, RSS feeds, web pages, Twitter queries, and Composio
 * integrations. Polls `openhuman.memory_sources_status_list` every 5s
 * for per-source chunk counts and freshness. The Sync button on each
 * row dispatches `openhuman.memory_sources_sync` which runs in the
 * background and emits MemorySyncStageChanged events.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type FreshnessLabel,
  listMemorySources,
  type MemorySourceEntry,
  memorySourcesStatusList,
  removeMemorySource,
  SOURCE_KIND_ICONS,
  SOURCE_KIND_LABEL_KEYS,
  type SourceStatus,
  syncMemorySource,
  updateMemorySource,
} from '../../services/memorySourcesService';
import type { ToastNotification } from '../../types/intelligence';
import { AddMemorySourceDialog } from './AddMemorySourceDialog';

interface MemorySourcesRegistryProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
  pollIntervalMs?: number;
}

export function MemorySourcesRegistry({
  onToast,
  pollIntervalMs = 5000,
}: MemorySourcesRegistryProps) {
  const { t } = useT();
  const [sources, setSources] = useState<MemorySourceEntry[]>([]);
  const [statuses, setStatuses] = useState<SourceStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [syncingId, setSyncingId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [list, stats] = await Promise.all([
        listMemorySources().catch(err => {
          console.warn('[ui-flow][memory-sources] list failed', err);
          return [] as MemorySourceEntry[];
        }),
        memorySourcesStatusList().catch(err => {
          console.warn('[ui-flow][memory-sources] status_list failed', err);
          return [] as SourceStatus[];
        }),
      ]);
      setSources(list);
      setStatuses(stats);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!pollIntervalMs) return undefined;
    const id = setInterval(() => {
      void refresh();
    }, pollIntervalMs);
    return () => clearInterval(id);
  }, [pollIntervalMs, refresh]);

  const statusById = useMemo(() => {
    const m = new Map<string, SourceStatus>();
    for (const s of statuses) m.set(s.source_id, s);
    return m;
  }, [statuses]);

  const handleToggle = useCallback(
    async (source: MemorySourceEntry) => {
      try {
        const updated = await updateMemorySource(source.id, { enabled: !source.enabled });
        setSources(prev => prev.map(s => (s.id === updated.id ? updated : s)));
      } catch (err) {
        onToast?.({
          type: 'error',
          title: t('memorySources.toggleFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [onToast, t]
  );

  const handleRemove = useCallback(
    async (source: MemorySourceEntry) => {
      try {
        await removeMemorySource(source.id);
        setSources(prev => prev.filter(s => s.id !== source.id));
        onToast?.({ type: 'success', title: t('memorySources.removed'), message: source.label });
      } catch (err) {
        onToast?.({
          type: 'error',
          title: t('memorySources.removeFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [onToast, t]
  );

  const handleSync = useCallback(
    async (source: MemorySourceEntry) => {
      setSyncingId(source.id);
      try {
        await syncMemorySource(source.id);
        onToast?.({
          type: 'success',
          title: `${t('memorySources.sync.successTitle')} ${source.label}`,
          message: t('memorySources.sync.successMessage'),
        });
        void refresh();
      } catch (err) {
        onToast?.({
          type: 'error',
          title: `${t('memorySources.sync.failedTitle')} ${source.label}`,
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSyncingId(prev => (prev === source.id ? null : prev));
      }
    },
    [onToast, refresh, t]
  );

  const handleAdded = useCallback(
    (source: MemorySourceEntry) => {
      setSources(prev => [...prev, source]);
      onToast?.({ type: 'success', title: t('memorySources.added'), message: source.label });
      void refresh();
    },
    [onToast, refresh, t]
  );

  return (
    <section
      className="rounded-lg border border-stone-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900"
      data-testid="memory-sources">
      <header className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
          {t('memorySources.title')}
        </h3>
        <button
          type="button"
          onClick={() => setDialogOpen(true)}
          className="inline-flex items-center gap-1 rounded-md bg-primary-500 px-3 py-1.5
                     text-xs font-semibold text-white shadow-sm transition-colors
                     hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-200">
          <PlusIcon />
          {t('memorySources.addSource')}
        </button>
      </header>

      {loading ? (
        <p className="text-xs text-stone-500 dark:text-neutral-400">{t('common.loading')}</p>
      ) : sources.length === 0 ? (
        <p className="text-xs text-stone-500 dark:text-neutral-400">{t('memorySources.empty')}</p>
      ) : (
        <ul className="divide-y divide-stone-100 dark:divide-neutral-800">
          {sources.map(source => (
            <SourceRow
              key={source.id}
              source={source}
              status={statusById.get(source.id) ?? null}
              isSyncing={syncingId === source.id}
              onToggle={handleToggle}
              onRemove={handleRemove}
              onSync={handleSync}
            />
          ))}
        </ul>
      )}

      <AddMemorySourceDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        onAdded={handleAdded}
      />
    </section>
  );
}

interface SourceRowProps {
  source: MemorySourceEntry;
  status: SourceStatus | null;
  isSyncing: boolean;
  onToggle: (source: MemorySourceEntry) => void;
  onRemove: (source: MemorySourceEntry) => void;
  onSync: (source: MemorySourceEntry) => void;
}

function SourceRow({ source, status, isSyncing, onToggle, onRemove, onSync }: SourceRowProps) {
  const { t } = useT();
  const icon = SOURCE_KIND_ICONS[source.kind] ?? '📄';
  const kindLabel = t(SOURCE_KIND_LABEL_KEYS[source.kind] ?? source.kind);
  const detail = sourceDetail(source);
  const lastSync = status ? relativeTimestamp(status.last_chunk_at_ms, t) : null;

  return (
    <li
      className="flex flex-col gap-2 py-3 sm:flex-row sm:items-start sm:justify-between"
      data-testid={`memory-source-row-${source.kind}`}>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-base">{icon}</span>
          <span
            className={`truncate text-sm font-medium ${
              source.enabled
                ? 'text-stone-900 dark:text-neutral-100'
                : 'text-stone-400 line-through dark:text-neutral-500'
            }`}>
            {source.label}
          </span>
          <span className="rounded-md bg-stone-100 px-1.5 py-0.5 text-[10px] font-medium text-stone-500 dark:bg-neutral-800 dark:text-neutral-400">
            {kindLabel}
          </span>
          {status && status.chunks_synced > 0 && <FreshnessPill freshness={status.freshness} />}
        </div>
        {detail && (
          <p className="mt-0.5 truncate pl-7 text-xs text-stone-400 dark:text-neutral-500">
            {detail}
          </p>
        )}
        {status && (status.chunks_synced > 0 || status.chunks_pending > 0) && (
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 pl-7 text-xs text-stone-500 dark:text-neutral-400">
            <span>
              {status.chunks_synced.toLocaleString()} {t('sync.chunks')}
            </span>
            {lastSync && (
              <span>
                {t('sync.lastChunk')} {lastSync}
              </span>
            )}
            {status.chunks_pending > 0 && (
              <span>
                {status.chunks_pending.toLocaleString()} {t('sync.pending')}
              </span>
            )}
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={() => onSync(source)}
          disabled={!source.enabled || isSyncing}
          title={t('sync.sync')}
          data-testid={`memory-source-sync-${source.toolkit ?? source.kind}`}
          className="inline-flex items-center gap-1 rounded-md bg-primary-500 px-3 py-1.5
                     text-xs font-semibold text-white shadow-sm transition-colors
                     hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50
                     focus:outline-none focus:ring-2 focus:ring-primary-200">
          {isSyncing ? <Spinner /> : <SyncIcon />}
          {isSyncing ? t('sync.syncing') : t('sync.sync')}
        </button>
        <button
          type="button"
          onClick={() => onToggle(source)}
          title={source.enabled ? t('memorySources.disable') : t('memorySources.enable')}
          className={`relative h-5 w-9 rounded-full transition-colors ${
            source.enabled ? 'bg-primary-500' : 'bg-stone-300 dark:bg-neutral-600'
          }`}>
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform ${
              source.enabled ? 'left-[18px]' : 'left-0.5'
            }`}
          />
        </button>
        <button
          type="button"
          onClick={() => onRemove(source)}
          title={t('memorySources.remove')}
          className="rounded p-1 text-stone-400 transition-colors hover:bg-coral-50
                     hover:text-coral-600 dark:text-neutral-500 dark:hover:bg-coral-500/10
                     dark:hover:text-coral-400">
          <TrashIcon />
        </button>
      </div>
    </li>
  );
}

function FreshnessPill({ freshness }: { freshness: FreshnessLabel }) {
  const { t } = useT();
  const label =
    freshness === 'active'
      ? t('sync.active')
      : freshness === 'recent'
        ? t('sync.recent')
        : t('sync.idle');
  const cls =
    freshness === 'active'
      ? 'bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300'
      : freshness === 'recent'
        ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
        : 'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200';
  return <span className={`rounded-md px-2 py-0.5 text-[10px] font-medium ${cls}`}>{label}</span>;
}

function relativeTimestamp(epochMs: number | null, t: (k: string) => string): string | null {
  if (epochMs === null) return null;
  const delta = Date.now() - epochMs;
  if (delta < 1000) return t('time.justNow');
  const seconds = Math.floor(delta / 1000);
  if (seconds < 60) return `${seconds}${t('time.secondsAgoSuffix')}`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}${t('time.minutesAgoSuffix')}`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}${t('time.hoursAgoSuffix')}`;
  const days = Math.floor(hours / 24);
  return `${days}${t('time.daysAgoSuffix')}`;
}

function sourceDetail(source: MemorySourceEntry): string | null {
  switch (source.kind) {
    case 'composio': {
      const parts = [source.toolkit, source.connection_id].filter(Boolean);
      return parts.length ? parts.join(' · ') : null;
    }
    case 'folder':
      return source.path ?? null;
    case 'github_repo':
      return source.url ?? null;
    case 'rss_feed':
      return source.url ?? null;
    case 'web_page':
      return source.url ?? null;
    case 'twitter_query':
      return source.query ?? null;
    default:
      return null;
  }
}

function PlusIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6" />
    </svg>
  );
}

function SyncIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M21 12a9 9 0 11-3-6.7" />
      <path d="M21 4v5h-5" />
    </svg>
  );
}

function Spinner() {
  return (
    <svg
      className="animate-spin"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden="true">
      <circle cx="12" cy="12" r="9" opacity="0.25" />
      <path d="M21 12a9 9 0 00-9-9" />
    </svg>
  );
}
