import Badge, { type BadgeVariant } from '../../../components/ui/Badge';
import { useT } from '../../../lib/i18n/I18nContext';
import type { CoreCronJob, CoreCronSchedule } from '../../../utils/tauriCommands/cron';
import type { MemorySyncStatusRow } from '../../../utils/tauriCommands/memoryTree';
import type { MemorySyncSummary } from '../hooks/useBackgroundActivity';
import { formatRelativeTime, formatResetTime } from '../utils/format';

/** Small, grey section divider shared across the background-activity sections. */
export function SectionHeader({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="flex items-center justify-between px-2.5 pb-1 pt-3">
      <span className="text-[11px] font-semibold uppercase tracking-wide text-content-faint">
        {title}
      </span>
      {hint ? <span className="text-[11px] text-content-faint">{hint}</span> : null}
    </div>
  );
}

/** A coloured status dot. */
function Dot({ className }: { className: string }) {
  return <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${className}`} />;
}

/** Localised, human-readable summary of a cron schedule. */
function scheduleLabel(schedule: CoreCronSchedule, t: ReturnType<typeof useT>['t']): string {
  switch (schedule.kind) {
    case 'cron':
      return t('conversations.backgroundTasks.cronSchedCron').replace('{expr}', schedule.expr);
    case 'every':
      return t('conversations.backgroundTasks.cronSchedEvery').replace(
        '{duration}',
        formatDuration(schedule.every_ms)
      );
    case 'at':
      return t('conversations.backgroundTasks.cronSchedAt');
    default:
      return '';
  }
}

function formatDuration(ms: number): string {
  const mins = Math.round(ms / 60_000);
  if (mins < 60) return `${mins}m`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

/** One read-only scheduled (cron) job. */
export function CronJobRow({ job }: { job: CoreCronJob }) {
  const { t } = useT();
  const name =
    (job.name && job.name.trim()) ||
    (job.prompt && job.prompt.trim()) ||
    (job.command && job.command.trim()) ||
    t('conversations.backgroundTasks.cronUnnamed');

  // `coral`, not a raw `red-*` scale: a raw palette value does not follow a
  // user's custom theme, and every other failure surface here is coral.
  const lastDot =
    job.last_status === 'error'
      ? 'bg-coral-500'
      : job.last_status === 'ok'
        ? 'bg-sage-500'
        : 'bg-surface-strong';

  const lastLabel = job.last_run
    ? t('conversations.backgroundTasks.cronLast').replace(
        '{time}',
        formatRelativeTime(job.last_run)
      )
    : t('conversations.backgroundTasks.cronNever');

  return (
    <div
      data-testid="background-cron-row"
      className={`mb-1 flex items-start gap-2.5 rounded-lg px-2.5 py-2 ${
        job.enabled ? '' : 'opacity-50'
      }`}>
      <Dot className={lastDot} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-sm font-medium text-content">{name}</span>
          {job.enabled ? (
            <span className="shrink-0 text-[11px] text-content-faint">
              {job.next_run
                ? t('conversations.backgroundTasks.cronNext').replace(
                    '{time}',
                    formatResetTime(job.next_run)
                  )
                : ''}
            </span>
          ) : (
            <Badge className="shrink-0 rounded-full">
              {t('conversations.backgroundTasks.cronPaused')}
            </Badge>
          )}
        </div>
        <span className="mt-0.5 block truncate text-[12px] text-content-muted">
          {scheduleLabel(job.schedule, t)}
        </span>
        <span className="mt-0.5 block text-[11px] text-content-faint">{lastLabel}</span>
      </div>
    </div>
  );
}

/**
 * Per-provider status pill, driven *only* by freshness (recency of the last
 * ingested chunk). Deliberately NOT keyed off `batch_total > batch_processed`:
 * an incomplete embedding wave can sit un-drained for days, and treating that
 * as "Syncing now" falsely implies live activity. A stalled backlog is
 * surfaced separately as a muted progress hint — see {@link MemorySection}.
 */
function providerFreshnessLabel(
  row: MemorySyncStatusRow,
  t: ReturnType<typeof useT>['t']
): { dot: string; label: string; variant: BadgeVariant } {
  if (row.freshness === 'active') {
    return {
      dot: 'bg-amber-500 animate-pulse',
      label: t('conversations.backgroundTasks.memProviderActive'),
      variant: 'warning',
    };
  }
  if (row.freshness === 'recent') {
    return {
      dot: 'bg-sage-500',
      label: t('conversations.backgroundTasks.memProviderRecent'),
      variant: 'success',
    };
  }
  return {
    dot: 'bg-surface-strong',
    label: t('conversations.backgroundTasks.memProviderIdle'),
    variant: 'neutral',
  };
}

/** Memory ingestion worker row + per-provider freshness rows. */
export function MemorySection({ memory }: { memory: MemorySyncSummary }) {
  const { t } = useT();
  const hasActivity = memory.ingesting || memory.queueDepth > 0 || memory.providers.length > 0;

  if (!hasActivity) {
    return (
      <div className="px-2.5 py-2 text-[12px] text-content-faint">
        {t('conversations.backgroundTasks.memUpToDate')}
      </div>
    );
  }

  return (
    <div>
      {memory.ingesting ? (
        <div
          data-testid="background-memory-ingesting"
          className="mb-1 flex items-start gap-2.5 rounded-lg px-2.5 py-2">
          <Dot className="bg-amber-500 animate-pulse" />
          <div className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium text-content">
              {memory.currentTitle
                ? t('conversations.backgroundTasks.memIngesting').replace(
                    '{title}',
                    memory.currentTitle
                  )
                : t('conversations.backgroundTasks.memIngestingUntitled')}
            </span>
            {memory.queueDepth > 0 ? (
              <span className="mt-0.5 block text-[11px] text-content-faint">
                {t('conversations.backgroundTasks.memQueued').replace(
                  '{count}',
                  String(memory.queueDepth)
                )}
              </span>
            ) : null}
          </div>
        </div>
      ) : null}

      {memory.providers.map(row => {
        const f = providerFreshnessLabel(row, t);
        // An incomplete embedding wave that is NOT live (freshness !== active):
        // a backlog the index worker hasn't drained, shown as muted progress —
        // never as "Syncing now".
        const backlog =
          row.freshness !== 'active' && row.batch_total > row.batch_processed
            ? `${row.batch_processed}/${row.batch_total} indexed`
            : null;
        return (
          <div
            key={row.provider}
            data-testid="background-memory-provider-row"
            className="mb-1 flex items-start gap-2.5 rounded-lg px-2.5 py-2">
            <Dot className={f.dot} />
            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-sm font-medium capitalize text-content">
                  {row.provider}
                </span>
                <Badge variant={f.variant} className="shrink-0 rounded-full">
                  {f.label}
                </Badge>
              </div>
              {backlog ? (
                <span className="mt-0.5 block text-[11px] text-content-faint">{backlog}</span>
              ) : null}
            </div>
          </div>
        );
      })}
    </div>
  );
}
