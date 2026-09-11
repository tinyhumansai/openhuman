/**
 * A single row in `MemorySourcesRegistry`'s source list: label + kind badge +
 * freshness pill, live sync progress / terminal result, the GH-4690 pipeline
 * warning banner, and the settings/sync/build/enable/remove controls.
 *
 * Split out of `MemorySourcesRegistry.tsx` to keep that file under the size
 * budget.
 */
import { useT } from '../../lib/i18n/I18nContext';
import {
  type FreshnessLabel,
  type MemorySourceEntry,
  SOURCE_KIND_ICONS,
  SOURCE_KIND_LABEL_KEYS,
  type SourceStatus,
} from '../../services/memorySourcesService';
import type { ToastNotification } from '../../types/intelligence';
import type {
  BackfillStatus,
  MemoryTreePipelineStatus,
} from '../../utils/tauriCommands/memoryTree';
import Button from '../ui/Button';
import { CollapsibleContent, CollapsibleRoot, CollapsibleTrigger } from '../ui/Collapsible';
import Switch from '../ui/Switch';
import {
  BuildIcon,
  CheckIcon,
  GearIcon,
  Spinner,
  SyncIcon,
  TrashIcon,
  WarnIcon,
} from './memorySourcesIcons';
import { relativeTimestamp, sourceDetail } from './memorySourcesRowHelpers';
import {
  STAGE_FALLBACK_PERCENT,
  type SyncProgress,
  type SyncResult,
} from './memorySourcesSyncTypes';
import {
  deriveSourcePipelineHealth,
  pipelineIssueMessageKey,
  type SourcePipelineHealth,
  VECTORS_PENDING_MESSAGE_KEY,
} from './sourcePipelineStatus';
import { SourceSettingsPanel } from './SourceSettingsPanel';

interface SourceRowProps {
  source: MemorySourceEntry;
  status: SourceStatus | null;
  /** Global downstream pipeline health (GH-4690); `null` before first poll. */
  pipeline: MemoryTreePipelineStatus | null;
  /**
   * Global embed-backfill snapshot (openhuman#6025); `null` before the first
   * poll or when that RPC failed. Tells a backlog that is being drained from
   * one nothing is working on.
   */
  backfill: BackfillStatus | null;
  /** Whether a backend session exists — gates the "Sign in to enable" prompt. */
  isAuthenticated: boolean;
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
  /** Navigate to Brain > Memory > Sync (the full memory-health panel). */
  onViewHealth: () => void;
  /** Navigate to sign-in (embeddings need a backend session). */
  onSignIn: () => void;
}

/**
 * The line shown beside a successful count when the run stopped short. With a
 * zero count the chip itself says why the run stopped (there is no count to
 * show instead), so the note is never repeated beside it.
 */
function syncNoteKey(result: SyncResult): string | null {
  if (!(result.items && result.items > 0)) return null;
  if (result.note === 'more_pending') return 'memorySources.sync.morePending';
  if (result.note === 'budget_spent') return 'memorySources.sync.budgetSpent';
  return null;
}

export function MemorySourceRow({
  source,
  status,
  pipeline,
  backfill,
  isAuthenticated,
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
  onViewHealth,
  onSignIn,
}: SourceRowProps) {
  const { t } = useT();
  const icon = SOURCE_KIND_ICONS[source.kind] ?? '📄';
  const kindLabel = t(SOURCE_KIND_LABEL_KEYS[source.kind] ?? source.kind);
  const detail = sourceDetail(source);
  const lastSync = status ? relativeTimestamp(status.last_chunk_at_ms, t) : null;

  // GH-4690: layered pipeline verdict. Only meaningful once no sync is
  // reporting live progress — mid-run the row renders the progress bar
  // instead. A terminal result chip does NOT suppress it (openhuman#6025):
  // the chip sits on the row until the next sync starts, and the minutes
  // right after `completed` are exactly when the embed backlog is visible.
  // The verdict itself tells a draining backlog (neutral note) from a stuck
  // one (amber warning), so it no longer needs the chip as a proxy.
  const settled = !progress;
  const noteKey = result ? syncNoteKey(result) : null;
  const health: SourcePipelineHealth = settled
    ? deriveSourcePipelineHealth(status, pipeline, backfill)
    : { state: 'none', issues: [], authRelated: false, vectorsPending: false };
  const ingestedOnly = health.state === 'ingested_only';
  if (ingestedOnly) {
    console.debug(
      `[ui-flow][memory-sources] source=${source.id} ingested-only issues=${health.issues.join(',')}`
    );
  }
  if (health.vectorsPending) {
    console.debug(
      `[ui-flow][memory-sources] source=${source.id} vectors-pending chunks=${status?.chunks_pending ?? 0}`
    );
  }

  return (
    <li className="py-3" data-testid={`memory-source-row-${source.kind}`}>
      {/* Per-row settings panel is a single disclosure region controlled by
          the registry (`settingsExpanded` + `onToggleSettings`) — Collapsible,
          not Accordion: there is no sibling exclusivity to coordinate here. */}
      <CollapsibleRoot
        open={settingsExpanded}
        onOpenChange={() => onToggleSettings(source.id)}
        className="flex flex-col gap-2">
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
              {status &&
                status.chunks_synced > 0 &&
                (ingestedOnly ? (
                  <IngestedOnlyPill sourceId={source.id} />
                ) : (
                  <FreshnessPill freshness={status.freshness} />
                ))}
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
                  <>
                    <span className="inline-flex items-center gap-1 rounded-md bg-sage-100 px-2 py-0.5 text-xs font-medium text-sage-700 dark:bg-sage-500/20 dark:text-sage-300">
                      <CheckIcon />
                      {result.items && result.items > 0
                        ? `${result.items.toLocaleString()} ${t('memorySources.sync.itemsSynced')}`
                        : result.note === 'budget_spent'
                          ? t('memorySources.sync.budgetSpent')
                          : result.note === 'more_pending'
                            ? t('memorySources.sync.morePending')
                            : t('memorySources.sync.upToDate')}
                    </span>
                    {noteKey && (
                      <span
                        className="ml-2 text-xs text-content-muted"
                        data-testid={`memory-source-note-${source.id}`}>
                        {t(noteKey)}
                      </span>
                    )}
                  </>
                ) : (
                  <span
                    className="inline-flex items-start gap-1 rounded-md bg-coral-50 px-2 py-0.5 text-xs font-medium text-coral-700 dark:bg-coral-500/10 dark:text-coral-300"
                    title={result.reason ?? undefined}>
                    <WarnIcon />
                    <span className="wrap-break-word">
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
            {health.vectorsPending && status && (
              <p
                className="mt-1 pl-7 text-xs text-content-muted"
                data-testid={`memory-source-vectors-pending-${source.id}`}
                role="status">
                {t(VECTORS_PENDING_MESSAGE_KEY).replace(
                  '{count}',
                  status.chunks_pending.toLocaleString()
                )}
              </p>
            )}
            {ingestedOnly && (
              <div
                className="mt-2 flex flex-col gap-1 rounded-md border border-amber-200 bg-amber-50 px-2.5 py-1.5 pl-7 text-xs text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200"
                data-testid={`memory-source-pipeline-warning-${source.id}`}
                role="status">
                {health.issues.map(issue => (
                  <div key={issue} className="flex items-start gap-1.5">
                    <WarnIcon />
                    <span className="wrap-break-word">{t(pipelineIssueMessageKey(issue))}</span>
                  </div>
                ))}
                <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 pl-[18px]">
                  {health.authRelated && !isAuthenticated && (
                    <Button
                      variant="tertiary"
                      size="xs"
                      onClick={onSignIn}
                      data-testid={`memory-source-signin-${source.id}`}
                      className="h-auto px-0 font-medium text-amber-900 underline underline-offset-2 hover:bg-transparent hover:text-amber-950 dark:text-amber-100 dark:hover:text-content-inverted">
                      {t('sync.pipeline.signInToEnable')}
                    </Button>
                  )}
                  <Button
                    variant="tertiary"
                    size="xs"
                    onClick={onViewHealth}
                    data-testid={`memory-source-view-health-${source.id}`}
                    className="h-auto px-0 font-medium text-amber-900 underline underline-offset-2 hover:bg-transparent hover:text-amber-950 dark:text-amber-100 dark:hover:text-content-inverted">
                    {t('sync.pipeline.viewHealth')}
                  </Button>
                </div>
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <CollapsibleTrigger asChild>
              <Button
                iconOnly
                variant="tertiary"
                size="xs"
                title={t('memorySources.settings.button')}
                aria-label={t('memorySources.settings.button')}
                data-testid={`memory-source-settings-${source.id}`}
                className={
                  settingsExpanded
                    ? 'bg-primary-100 text-primary-600 dark:bg-primary-500/20 dark:text-primary-400'
                    : 'text-content-faint hover:text-content-secondary'
                }>
                <GearIcon />
              </Button>
            </CollapsibleTrigger>
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
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onBuild(source)}
              disabled={!source.enabled || isBuilding || isSyncing}
              title={t('memorySources.build.title')}
              leadingIcon={isBuilding ? <Spinner /> : <BuildIcon />}
              className="border-primary-300 text-primary-600 hover:bg-primary-50
                     dark:border-primary-500/30 dark:text-primary-400 dark:hover:bg-primary-500/10">
              {isBuilding ? t('memorySources.build.building') : t('memorySources.build.title')}
            </Button>
            <Switch
              id={`memory-source-toggle-${source.id}`}
              checked={source.enabled}
              onCheckedChange={() => onToggle(source)}
              aria-label={source.enabled ? t('memorySources.disable') : t('memorySources.enable')}
            />
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
        <CollapsibleContent>
          <SourceSettingsPanel source={source} onSaved={onSettingsSaved} onToast={onToast} />
        </CollapsibleContent>
      </CollapsibleRoot>
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

/**
 * GH-4690: the "Ingested only" warning pill. Replaces the clean freshness pill
 * when a source ingested chunks but the downstream retrieval pipeline failed
 * (no vectors / extraction / degraded tree). Amber, matching the degraded
 * badges in the Brain > Memory > Sync panel, so raw sync ≠ retrieval-ready is
 * unmistakable at a glance.
 */
function IngestedOnlyPill({ sourceId }: { sourceId: string }) {
  const { t } = useT();
  return (
    <span
      className="rounded-md bg-amber-100 px-2 py-0.5 text-[10px] font-medium text-amber-800 dark:bg-amber-500/20 dark:text-amber-200"
      data-testid={`memory-source-ingested-only-${sourceId}`}>
      {t('sync.pipeline.ingestedOnly')}
    </span>
  );
}
