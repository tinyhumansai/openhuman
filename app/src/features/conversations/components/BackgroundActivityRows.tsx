import { useT } from '../../../lib/i18n/I18nContext';
import type { CoreCronJob, CoreCronSchedule } from '../../../utils/tauriCommands/cron';
import type { MemorySyncStatusRow } from '../../../utils/tauriCommands/memoryTree';
import type { MemorySyncSummary, SubconsciousSummary } from '../hooks/useBackgroundActivity';
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

  const lastDot =
    job.last_status === 'error'
      ? 'bg-red-500'
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
            <span className="shrink-0 text-[11px] font-medium text-content-muted">
              {t('conversations.backgroundTasks.cronPaused')}
            </span>
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

/** Single status row for the subconscious / background-thinking loop. */
export function SubconsciousRow({ summary }: { summary: SubconsciousSummary }) {
  const { t } = useT();
  const off = !summary.enabled || summary.mode === 'off';

  let dot: string;
  let pill: string;
  let pillClass: string;
  if (off) {
    dot = 'bg-surface-strong';
    pill = t('conversations.backgroundTasks.subOff');
    pillClass = 'text-content-faint';
  } else if (summary.working) {
    dot = 'bg-amber-500 animate-pulse';
    pill = t('conversations.backgroundTasks.subWorking');
    pillClass = 'text-amber-700 dark:text-amber-300';
  } else {
    dot = 'bg-sage-500';
    pill = t('conversations.backgroundTasks.subIdle');
    pillClass = 'text-sage-700 dark:text-sage-300';
  }

  const lastLabel =
    summary.lastTickAt != null
      ? t('conversations.backgroundTasks.subLastRan').replace(
          '{time}',
          // last_tick_at is epoch *seconds*; formatRelativeTime wants a date string.
          formatRelativeTime(new Date(summary.lastTickAt * 1000).toISOString())
        )
      : t('conversations.backgroundTasks.subNeverRan');

  const meta = [
    lastLabel,
    t('conversations.backgroundTasks.subTicks').replace('{count}', String(summary.totalTicks)),
    summary.queueDepth && summary.queueDepth > 0
      ? t('conversations.backgroundTasks.subQueued').replace('{count}', String(summary.queueDepth))
      : null,
  ].filter(Boolean);

  return (
    <div
      data-testid="background-subconscious-row"
      className={`mb-1 flex items-start gap-2.5 rounded-lg px-2.5 py-2 ${off ? 'opacity-60' : ''}`}>
      <Dot className={dot} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-sm font-medium text-content">
            {t('conversations.backgroundTasks.sectionSubconscious')}
          </span>
          <span className={`shrink-0 text-[11px] font-medium ${pillClass}`}>{pill}</span>
        </div>
        <span className="mt-0.5 block text-[11px] text-content-faint">{meta.join(' · ')}</span>
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
): { dot: string; label: string; pillClass: string } {
  if (row.freshness === 'active') {
    return {
      dot: 'bg-amber-500 animate-pulse',
      label: t('conversations.backgroundTasks.memProviderActive'),
      pillClass: 'text-amber-700 dark:text-amber-300',
    };
  }
  if (row.freshness === 'recent') {
    return {
      dot: 'bg-sage-500',
      label: t('conversations.backgroundTasks.memProviderRecent'),
      pillClass: 'text-sage-700 dark:text-sage-300',
    };
  }
  return {
    dot: 'bg-surface-strong',
    label: t('conversations.backgroundTasks.memProviderIdle'),
    pillClass: 'text-content-faint',
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
                <span className={`shrink-0 text-[11px] font-medium ${f.pillClass}`}>{f.label}</span>
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
