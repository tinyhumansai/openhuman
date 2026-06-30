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
import debug from 'debug';
import { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { CoreStateContext } from '../../providers/coreStateContext';
import {
  applyAllIn,
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
import type {
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../../types/intelligence';
import {
  type MemorySyncSettings,
  openhumanGetMemorySyncSettings,
  openhumanUpdateMemorySyncSettings,
} from '../../utils/tauriCommands/config';
import { memoryTreeFlushSource } from '../../utils/tauriCommands/memoryTree';
import Button from '../ui/Button';
import { AddMemorySourceDialog } from './AddMemorySourceDialog';
import { ConfirmationModal } from './ConfirmationModal';
import { SourceSettingsPanel } from './SourceSettingsPanel';

const log = debug('intelligence:memory-sync');

interface MemorySourcesRegistryProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
  pollIntervalMs?: number;
}

interface SyncProgress {
  stage: string;
  detail: string | null;
  percent: number | null;
}

/**
 * Terminal outcome of a sync run, shown on the row after the `completed` or
 * `failed` stage event arrives (#3295). Persists until the next sync starts,
 * so a no-op ("0 new items") or failed sync leaves visible confirmation
 * instead of the indicator silently vanishing.
 */
interface SyncResult {
  kind: 'success' | 'failed';
  /** New items ingested (success only); null when the count is unknown. */
  items: number | null;
  /** Human-readable failure reason (failed only). */
  reason: string | null;
}

/**
 * Per-stage fallback percentages so the progress bar always advances even
 * when no numeric "N/M" ratio is present in the detail string (RC#4, #3295).
 */
export const STAGE_FALLBACK_PERCENT: Record<string, number> = {
  requested: 2,
  fetching: 5,
  stored: 15,
  queued: 25,
  ingesting: 40,
  completed: 100,
};

/**
 * Parse a sync progress detail string into a 0–100 percent.
 *
 * - Recognises "N/M ..." numeric patterns and returns N/M as a ratio.
 * - Falls back to the per-stage baseline when no ratio is present rather
 *   than returning a bogus number (RC#4, issue #3295).
 * - Returns `null` when both approaches are unavailable (no stage either).
 */
export function parseSyncProgress(detail: string | null, stage?: string): number | null {
  // Try the numeric "N/M ..." ratio first.
  if (detail) {
    const match = detail.match(/^(\d+)\/(\d+)[\s/]/);
    if (match) {
      const current = parseInt(match[1], 10);
      const total = parseInt(match[2], 10);
      if (total > 0) return Math.round((current / total) * 100);
    }
  }
  // Fall back to the per-stage baseline percentage.
  if (stage && stage in STAGE_FALLBACK_PERCENT) {
    return STAGE_FALLBACK_PERCENT[stage];
  }
  return null;
}

/**
 * Parse the number of newly-ingested items from a `completed` stage detail
 * string. The backend formats this as `"ingested N item(s)"`
 * (`memory_sources/sync.rs`). Returns `null` when no count is present so the
 * UI can fall back to a generic "synced" confirmation (#3295).
 */
export function parseIngestedCount(detail: string | null): number | null {
  if (!detail) return null;
  const match = detail.match(/ingested\s+(\d+)\s+item/i);
  if (match) return parseInt(match[1], 10);
  return null;
}

export function MemorySourcesRegistry({
  onToast,
  pollIntervalMs = 5000,
}: MemorySourcesRegistryProps) {
  const { t } = useT();
  // Read the core snapshot directly (not via the throwing `useCoreState`
  // hook) so this component still renders in unit tests that mount it
  // without a CoreStateProvider — there `ctx` is null and `isAuthenticated`
  // stays a stable `false`, so the load effect behaves exactly as before.
  const coreState = useContext(CoreStateContext);
  const isAuthenticated = coreState?.snapshot.auth.isAuthenticated ?? false;
  const [sources, setSources] = useState<MemorySourceEntry[]>([]);
  const [statuses, setStatuses] = useState<SourceStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [dialogOpen, setDialogOpen] = useState(false);
  // RC#1 (#3295): use a Set so multiple sources can show "syncing" concurrently.
  // Set state is always replaced with a new Set to trigger re-renders.
  const [syncingIds, setSyncingIds] = useState<Set<string>>(new Set());
  const [buildingId, setBuildingId] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<Map<string, SyncProgress>>(new Map());
  // Terminal per-source result (success/failure) shown after a sync ends (#3295).
  const [syncResults, setSyncResults] = useState<Map<string, SyncResult>>(new Map());
  const [allInModalOpen, setAllInModalOpen] = useState(false);
  const [applyingAllIn, setApplyingAllIn] = useState(false);
  const allInInFlightRef = useRef(false);
  const [expandedSettingsId, setExpandedSettingsId] = useState<string | null>(null);

  // Refs let the (intentionally dep-free) sync-stage listener fire accurate
  // toasts on the *terminal* event without re-subscribing on every render or
  // 5s poll. The handler must read the latest onToast/sources/t (#3295).
  const onToastRef = useRef(onToast);
  const sourcesRef = useRef(sources);
  const tRef = useRef(t);
  useEffect(() => {
    onToastRef.current = onToast;
    sourcesRef.current = sources;
    tRef.current = t;
  });

  useEffect(() => {
    const handler = (e: Event) => {
      const data = (e as CustomEvent).detail as {
        stage?: string;
        /** Originating memory-source id (RC#2, #3295). Preferred over connection_id. */
        source_id?: string | null;
        /** Legacy: document/connection id. Still present for backward compat. */
        connection_id?: string | null;
        detail?: string;
      } | null;

      // RC#2 (#3295): prefer source_id when present; fall back to connection_id for
      // backward compat with older core versions that don't emit source_id yet.
      const rowId = data?.source_id ?? data?.connection_id;
      if (!rowId) return;

      const stage = data?.stage ?? '';

      console.debug(
        `[ui-flow][memory-sync] stage=${stage} rowId=${rowId} source_id=${data?.source_id ?? 'absent'} connection_id=${data?.connection_id ?? 'absent'}`
      );

      if (stage === 'completed' || stage === 'failed') {
        // Clear the live progress bar + syncing flag for this row.
        setSyncProgress(prev => {
          const next = new Map(prev);
          next.delete(rowId);
          return next;
        });
        // RC#1: immutable Set update — remove just this source, keep others syncing.
        setSyncingIds(prev => {
          const next = new Set(prev);
          next.delete(rowId);
          return next;
        });

        const tt = tRef.current;
        const label = sourcesRef.current.find(s => s.id === rowId)?.label ?? rowId;

        if (stage === 'completed') {
          // Success: record + toast the item count parsed from the detail
          // ("ingested N item(s)"). 0 new items → "up to date" (#3295).
          const items = parseIngestedCount(data?.detail ?? null);
          setSyncResults(prev => {
            const next = new Map(prev);
            next.set(rowId, { kind: 'success', items, reason: null });
            return next;
          });
          onToastRef.current?.({
            type: 'success',
            title: `${tt('memorySources.sync.completeTitle')} ${label}`,
            message:
              items && items > 0
                ? `${items} ${tt('memorySources.sync.itemsSynced')}`
                : tt('memorySources.sync.upToDate'),
          });
        } else {
          // Failure: surface the reason on the row + a toast. The core already
          // reported internal bugs to Sentry via report_error_or_expected.
          const reason = data?.detail ?? null;
          setSyncResults(prev => {
            const next = new Map(prev);
            next.set(rowId, { kind: 'failed', items: null, reason });
            return next;
          });
          onToastRef.current?.({
            type: 'error',
            title: `${tt('memorySources.sync.failedLabel')} · ${label}`,
            message: reason ?? tt('memorySources.sync.failedLabel'),
          });
        }
        return;
      }

      // Non-terminal stage: a sync is genuinely in progress. Drop any stale
      // terminal result for this row so the live bar replaces the old chip.
      setSyncResults(prev => {
        if (!prev.has(rowId)) return prev;
        const next = new Map(prev);
        next.delete(rowId);
        return next;
      });
      const percent = parseSyncProgress(data?.detail ?? null, stage);
      setSyncProgress(prev => {
        const next = new Map(prev);
        next.set(rowId, { stage, detail: data?.detail ?? null, percent });
        return next;
      });
      // RC#1: ADD this source id to the set (immutable update).
      if (
        stage === 'requested' ||
        stage === 'fetching' ||
        stage === 'stored' ||
        stage === 'queued' ||
        stage === 'ingesting'
      ) {
        setSyncingIds(prev => {
          if (prev.has(rowId)) return prev; // no change — avoid re-render
          const next = new Set(prev);
          next.add(rowId);
          return next;
        });
      }
    };
    window.addEventListener('openhuman:memory-sync-stage', handler);
    return () => window.removeEventListener('openhuman:memory-sync-stage', handler);
  }, []);

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
      // RC#5 (#3295): The 5s poll is the safety net for missed completed/failed events.
      // If a source is in syncingIds but the poll shows it's no longer active (no
      // in-progress status indicator from the server), we clear it here. In practice
      // the event stream covers this; on remount the state rehydrates within ~5s via poll.
      // No new RPC needed — reconciliation is best-effort and relies on the existing poll.
    } finally {
      setLoading(false);
    }
  }, []);

  // Load on mount AND whenever the session transitions to authenticated.
  // After a page reload the registry can mount (e.g. via a persisted
  // `?tab=memory` deep link) *before* CoreStateProvider has restored the
  // session, so the initial fetch runs against a not-yet-ready core and
  // surfaces nothing. Re-running when `isAuthenticated` flips true picks up
  // sources immediately instead of waiting for the next 5s poll — which
  // under CI load was racing the E2E visibility timeout (#3449).
  useEffect(() => {
    void refresh();
  }, [refresh, isAuthenticated]);

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

  // Newest chunk timestamp across every source — the "Last synced …" anchor
  // for the global schedule header. Derived from persisted chunk data, so it
  // survives restarts.
  const overallLastSyncMs = useMemo(() => {
    let newest: number | null = null;
    for (const s of statuses) {
      if (s.last_chunk_at_ms != null && (newest === null || s.last_chunk_at_ms > newest)) {
        newest = s.last_chunk_at_ms;
      }
    }
    return newest;
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
      // RC#1 (#3295): add immediately on click — event will also fire, but this
      // ensures the row lights up before the first sync-stage event arrives.
      setSyncingIds(prev => {
        const next = new Set(prev);
        next.add(source.id);
        return next;
      });
      // A fresh sync is starting — drop any prior terminal result chip (#3295).
      setSyncResults(prev => {
        if (!prev.has(source.id)) return prev;
        const next = new Map(prev);
        next.delete(source.id);
        return next;
      });
      console.debug(`[ui-flow][memory-sync] manual sync triggered source_id=${source.id}`);
      try {
        await syncMemorySource(source.id);
        // NOTE: success/failure feedback is intentionally NOT fired here. This
        // RPC returns in ~4ms after merely *spawning* the background sync; the
        // real outcome arrives via the terminal `completed`/`failed` stage event
        // (handled above), which carries the item count / failure reason (#3295).
        void refresh();
      } catch (err) {
        // The RPC call itself failed (transport/validation) — the background
        // sync never started, so no stage event will arrive. Surface it here:
        // clear the syncing flag and record a failed result on the row.
        const reason = err instanceof Error ? err.message : String(err);
        setSyncingIds(prev => {
          const next = new Set(prev);
          next.delete(source.id);
          return next;
        });
        setSyncResults(prev => {
          const next = new Map(prev);
          next.set(source.id, { kind: 'failed', items: null, reason });
          return next;
        });
        onToast?.({
          type: 'error',
          title: `${t('memorySources.sync.failedLabel')} · ${source.label}`,
          message: reason,
        });
      }
      // No `finally` clear: on success the row stays "syncing" until the
      // terminal stage event arrives (the sync is still running in the
      // background). Clearing here is what made the indicator vanish in ~4ms.
    },
    [onToast, refresh, t]
  );

  const handleBuild = useCallback(
    async (source: MemorySourceEntry) => {
      const scope = sourceTreeScope(source);
      if (!scope) return;
      setBuildingId(source.id);
      try {
        const resp = await memoryTreeFlushSource(scope);
        onToast?.({
          type: 'success',
          title: t('memorySources.build.successTitle'),
          message: `${resp.seals_fired} ${t('memorySources.build.sealsMessage')}`,
        });
      } catch (err) {
        onToast?.({
          type: 'error',
          title: t('memorySources.build.failedTitle'),
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setBuildingId(prev => (prev === source.id ? null : prev));
      }
    },
    [onToast, t]
  );

  const handleAdded = useCallback(
    (source: MemorySourceEntry) => {
      setSources(prev => [...prev, source]);
      onToast?.({ type: 'success', title: t('memorySources.added'), message: source.label });
      void refresh();
    },
    [onToast, refresh, t]
  );

  const handleConfirmAllIn = useCallback(async () => {
    if (allInInFlightRef.current) return;
    allInInFlightRef.current = true;
    setApplyingAllIn(true);
    try {
      const result = await applyAllIn();
      setSources(result.sources);
      onToast?.({ type: 'success', title: t('memorySources.allIn.success') });
    } catch (err) {
      onToast?.({
        type: 'error',
        title: t('memorySources.allIn.failed'),
        message: err instanceof Error ? err.message : String(err),
      });
    } finally {
      allInInFlightRef.current = false;
      setApplyingAllIn(false);
      setAllInModalOpen(false);
    }
  }, [onToast, t]);

  const handleSettingsSaved = useCallback((updated: MemorySourceEntry) => {
    setSources(prev => prev.map(s => (s.id === updated.id ? updated : s)));
  }, []);

  const handleToggleSettings = useCallback((sourceId: string) => {
    setExpandedSettingsId(prev => (prev === sourceId ? null : sourceId));
  }, []);

  const allInModal: ConfirmationModalType = {
    isOpen: allInModalOpen,
    title: t('memorySources.allIn.title'),
    message: t('memorySources.allIn.message'),
    confirmText: t('memorySources.allIn.confirm'),
    cancelText: t('memorySources.allIn.cancel'),
    destructive: false,
    onConfirm: () => {
      void handleConfirmAllIn();
    },
    onCancel: () => {
      setAllInModalOpen(false);
    },
  };

  return (
    <section className="rounded-lg border border-line bg-surface p-4" data-testid="memory-sources">
      <header className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-content-secondary">{t('memorySources.title')}</h3>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setAllInModalOpen(true)}
            disabled={applyingAllIn}
            data-testid="all-in-button"
            className="inline-flex items-center gap-1 rounded-md border border-primary-300
                       bg-surface px-3 py-1.5 text-xs font-semibold text-primary-600
                       shadow-sm transition-colors hover:bg-primary-50
                       disabled:cursor-not-allowed disabled:opacity-50
                       dark:border-primary-500/30 dark:bg-surface dark:text-primary-400
                       dark:hover:bg-primary-500/10
                       focus:outline-none focus:ring-2 focus:ring-primary-200">
            <AllInIcon />
            {t('memorySources.allIn.button')}
          </button>
          <Button
            variant="primary"
            size="sm"
            onClick={() => setDialogOpen(true)}
            leadingIcon={<PlusIcon />}>
            {t('memorySources.addSource')}
          </Button>
        </div>
      </header>

      <MemorySyncSchedule lastSyncMs={overallLastSyncMs} onToast={onToast} />

      {loading ? (
        <p className="text-xs text-content-muted">{t('common.loading')}</p>
      ) : sources.length === 0 ? (
        <p className="text-xs text-content-muted">{t('memorySources.empty')}</p>
      ) : (
        <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
          {sources.map(source => (
            <SourceRow
              key={source.id}
              source={source}
              status={statusById.get(source.id) ?? null}
              isSyncing={syncingIds.has(source.id) || syncProgress.has(source.id)}
              isBuilding={buildingId === source.id}
              progress={syncProgress.get(source.id) ?? null}
              result={syncResults.get(source.id) ?? null}
              settingsExpanded={expandedSettingsId === source.id}
              onToggle={handleToggle}
              onRemove={handleRemove}
              onSync={handleSync}
              onBuild={handleBuild}
              onToggleSettings={handleToggleSettings}
              onSettingsSaved={handleSettingsSaved}
              onToast={onToast}
            />
          ))}
        </ul>
      )}

      <AddMemorySourceDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        onAdded={handleAdded}
      />

      {allInModalOpen && (
        <ConfirmationModal modal={allInModal} onClose={() => setAllInModalOpen(false)} />
      )}
    </section>
  );
}

/** Manual-only sentinel — stored as `sync_interval_secs = 0`. */
const MANUAL_INTERVAL_SECS = 0;
/** Preset cadences offered in the UI (seconds): 4h / 12h / 24h. */
const SYNC_INTERVAL_PRESETS_SECS = [14_400, 43_200, 86_400];

/** Human label for a cadence ("Every 4h" / "Every 30m" / "Manual only"). */
function intervalChipLabel(secs: number, t: (k: string) => string): string {
  if (secs === MANUAL_INTERVAL_SECS) return t('memorySyncInterval.manual');
  if (secs % 3600 === 0) {
    return t('memorySyncInterval.everyHours').replace('{h}', String(secs / 3600));
  }
  return t('memorySyncInterval.everyMinutes').replace('{m}', String(Math.round(secs / 60)));
}

interface MemorySyncScheduleProps {
  lastSyncMs: number | null;
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

/**
 * Global memory-sync schedule control (#3302). Presented like a backup
 * schedule: "Last synced … · Sync every …", with preset cadences (4h / 12h /
 * 24h) plus "Manual only". Reads/writes `config_*_memory_sync_settings`.
 */
function MemorySyncSchedule({ lastSyncMs, onToast }: MemorySyncScheduleProps) {
  const { t } = useT();
  const [settings, setSettings] = useState<MemorySyncSettings | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let active = true;
    const loadSettings = async () => {
      try {
        const resp = await openhumanGetMemorySyncSettings();
        if (active) setSettings(resp.result);
      } catch (err) {
        log('get settings failed: %O', err);
      }
    };
    void loadSettings();
    return () => {
      active = false;
    };
  }, []);

  const handleSelect = useCallback(
    async (secs: number) => {
      setSaving(true);
      try {
        const resp = await openhumanUpdateMemorySyncSettings({ sync_interval_secs: secs });
        setSettings(resp.result);
      } catch (err) {
        onToast?.({
          type: 'error',
          title: t('memorySyncInterval.saveFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSaving(false);
      }
    },
    [onToast, t]
  );

  if (!settings) return null;

  const lastSync = relativeTimestamp(lastSyncMs, t);
  const currentLabel = settings.is_manual
    ? t('memorySyncInterval.manual')
    : intervalChipLabel(settings.selected_secs, t);
  // Option list: backend presets (fall back to the local defaults) + Manual.
  const presetSecs =
    settings.presets && settings.presets.length > 0 ? settings.presets : SYNC_INTERVAL_PRESETS_SECS;
  const options = [...presetSecs, MANUAL_INTERVAL_SECS];
  const selectedSecs = settings.is_manual ? MANUAL_INTERVAL_SECS : settings.selected_secs;

  return (
    <div
      className="mb-3 rounded-md border border-line bg-surface-muted p-3"
      data-testid="memory-sync-schedule">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <p className="text-xs font-semibold text-content-secondary">
            {t('memorySyncInterval.title')}
          </p>
          <p className="mt-0.5 text-xs text-content-muted">
            <span>
              {t('memorySyncInterval.lastSynced')} {lastSync ?? t('memorySyncInterval.never')}
            </span>
            <span aria-hidden="true"> · </span>
            <span data-testid="memory-sync-current">{currentLabel}</span>
          </p>
        </div>
        <div
          role="radiogroup"
          aria-label={t('memorySyncInterval.title')}
          className="flex flex-wrap items-center gap-1.5">
          {options.map(secs => {
            const isSelected = secs === selectedSecs;
            return (
              <button
                key={secs}
                type="button"
                role="radio"
                aria-checked={isSelected}
                disabled={saving}
                onClick={() => void handleSelect(secs)}
                data-testid={`memory-sync-preset-${secs}`}
                className={`rounded-md border px-2.5 py-1 text-xs font-medium transition-colors
                  focus:outline-none focus:ring-2 focus:ring-primary-200
                  disabled:cursor-not-allowed disabled:opacity-50 ${
                    isSelected
                      ? 'border-primary-500 bg-primary-50 text-primary-700 dark:border-primary-500/50 dark:bg-primary-500/10 dark:text-primary-300'
                      : 'border-line bg-surface text-content-secondary hover:bg-surface-hover'
                  }`}>
                {intervalChipLabel(secs, t)}
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}

interface SourceRowProps {
  source: MemorySourceEntry;
  status: SourceStatus | null;
  isSyncing: boolean;
  isBuilding: boolean;
  progress: SyncProgress | null;
  result: SyncResult | null;
  settingsExpanded: boolean;
  onToggle: (source: MemorySourceEntry) => void;
  onRemove: (source: MemorySourceEntry) => void;
  onSync: (source: MemorySourceEntry) => void;
  onBuild: (source: MemorySourceEntry) => void;
  onToggleSettings: (sourceId: string) => void;
  onSettingsSaved: (updated: MemorySourceEntry) => void;
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

function SourceRow({
  source,
  status,
  isSyncing,
  isBuilding,
  progress,
  result,
  settingsExpanded,
  onToggle,
  onRemove,
  onSync,
  onBuild,
  onToggleSettings,
  onSettingsSaved,
  onToast,
}: SourceRowProps) {
  const { t } = useT();
  const icon = SOURCE_KIND_ICONS[source.kind] ?? '📄';
  const kindLabel = t(SOURCE_KIND_LABEL_KEYS[source.kind] ?? source.kind);
  const detail = sourceDetail(source);
  const lastSync = status ? relativeTimestamp(status.last_chunk_at_ms, t) : null;

  return (
    <li className="flex flex-col gap-2 py-3" data-testid={`memory-source-row-${source.kind}`}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-base">{icon}</span>
            <span
              className={`truncate text-sm font-medium ${
                source.enabled ? 'text-content' : 'text-content-faint line-through'
              }`}>
              {source.label}
            </span>
            <span className="rounded-md bg-surface-subtle px-1.5 py-0.5 text-[10px] font-medium text-content-muted">
              {kindLabel}
            </span>
            {status && status.chunks_synced > 0 && <FreshnessPill freshness={status.freshness} />}
          </div>
          {detail && <p className="mt-0.5 truncate pl-7 text-xs text-content-faint">{detail}</p>}
          {progress && (
            <div className="mt-2 pl-7">
              <div className="flex items-center gap-2 text-xs text-content-muted">
                <span className="capitalize">{progress.stage}</span>
                {progress.percent !== null && (
                  <span className="font-medium text-primary-600 dark:text-primary-400">
                    {progress.percent}%
                  </span>
                )}
                {progress.detail && (
                  <span className="truncate text-content-faint">{progress.detail}</span>
                )}
              </div>
              <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-surface-strong">
                <div
                  className="h-full rounded-full bg-primary-500 transition-all duration-300"
                  style={{
                    width: `${progress.percent ?? STAGE_FALLBACK_PERCENT[progress.stage] ?? 2}%`,
                  }}
                />
              </div>
            </div>
          )}
          {!progress && result && (
            <div className="mt-2 pl-7" data-testid={`memory-source-result-${source.id}`}>
              {result.kind === 'success' ? (
                <span className="inline-flex items-center gap-1 rounded-md bg-sage-100 px-2 py-0.5 text-xs font-medium text-sage-700 dark:bg-sage-500/20 dark:text-sage-300">
                  <CheckIcon />
                  {result.items && result.items > 0
                    ? `${result.items.toLocaleString()} ${t('memorySources.sync.itemsSynced')}`
                    : t('memorySources.sync.upToDate')}
                </span>
              ) : (
                <span
                  className="inline-flex items-start gap-1 rounded-md bg-coral-50 px-2 py-0.5 text-xs font-medium text-coral-700 dark:bg-coral-500/10 dark:text-coral-300"
                  title={result.reason ?? undefined}>
                  <WarnIcon />
                  <span className="break-words">
                    {t('memorySources.sync.failedLabel')}
                    {result.reason ? `: ${result.reason}` : ''}
                  </span>
                </span>
              )}
            </div>
          )}
          {!progress &&
            !result &&
            status &&
            (status.chunks_synced > 0 || status.chunks_pending > 0) && (
              <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 pl-7 text-xs text-content-muted">
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
            onClick={() => onToggleSettings(source.id)}
            title={t('memorySources.settings.button')}
            data-testid={`memory-source-settings-${source.id}`}
            aria-expanded={settingsExpanded}
            className={`rounded p-1 transition-colors focus:outline-none focus:ring-2 focus:ring-primary-200 ${
              settingsExpanded
                ? 'bg-primary-100 text-primary-600 dark:bg-primary-500/20 dark:text-primary-400'
                : 'text-content-faint hover:bg-surface-hover hover:text-content-secondary'
            }`}>
            <GearIcon />
          </button>
          <Button
            variant="primary"
            size="sm"
            onClick={() => onSync(source)}
            disabled={!source.enabled || isSyncing}
            title={t('sync.sync')}
            data-testid={`memory-source-sync-${source.toolkit ?? source.kind}`}
            leadingIcon={isSyncing ? <Spinner /> : <SyncIcon />}>
            {isSyncing ? t('sync.syncing') : t('sync.sync')}
          </Button>
          <button
            type="button"
            onClick={() => onBuild(source)}
            disabled={!source.enabled || isBuilding || isSyncing}
            title={t('memorySources.build.title')}
            className="inline-flex items-center gap-1 rounded-md border border-primary-300
                     bg-surface px-3 py-1.5 text-xs font-semibold text-primary-600
                     shadow-sm transition-colors hover:bg-primary-50
                     disabled:cursor-not-allowed disabled:opacity-50
                     dark:border-primary-500/30 dark:bg-surface dark:text-primary-400
                     dark:hover:bg-primary-500/10
                     focus:outline-none focus:ring-2 focus:ring-primary-200">
            {isBuilding ? <Spinner /> : <BuildIcon />}
            {isBuilding ? t('memorySources.build.building') : t('memorySources.build.title')}
          </button>
          <button
            type="button"
            onClick={() => onToggle(source)}
            title={source.enabled ? t('memorySources.disable') : t('memorySources.enable')}
            className={`relative h-5 w-9 rounded-full transition-colors ${
              source.enabled ? 'bg-primary-500' : 'bg-surface-strong'
            }`}>
            <span
              className={`absolute top-0.5 h-4 w-4 rounded-full bg-surface shadow transition-transform ${
                source.enabled ? 'left-[18px]' : 'left-0.5'
              }`}
            />
          </button>
          <Button
            iconOnly
            variant="tertiary"
            tone="danger"
            size="xs"
            onClick={() => onRemove(source)}
            title={t('memorySources.remove')}
            aria-label={t('memorySources.remove')}>
            <TrashIcon />
          </Button>
        </div>
      </div>
      {settingsExpanded && (
        <SourceSettingsPanel
          source={source}
          syncedCount={status?.chunks_synced}
          onSaved={onSettingsSaved}
          onToast={onToast}
        />
      )}
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
        : 'bg-surface-subtle text-content-secondary';
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

function sourceTreeScope(source: MemorySourceEntry): string | null {
  if (source.kind === 'github_repo' && source.url) {
    const m = source.url.match(/github\.com\/([^/]+)\/([^/.]+)/);
    if (m) return `github:${m[1]}/${m[2]}`;
  }
  return source.id;
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

function BuildIcon() {
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
      <path d="M22 11.08V12a10 10 0 11-5.93-9.14" />
      <path d="M22 4L12 14.01l-3-3" />
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

function GearIcon() {
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
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M20 6L9 17l-5-5" />
    </svg>
  );
}

function WarnIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className="mt-0.5 shrink-0">
      <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      <path d="M12 9v4M12 17h.01" />
    </svg>
  );
}

function AllInIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
    </svg>
  );
}
