'use client';

/**
 * Replaces the legacy `BackgroundProcessesPanel` (deleted) with the vendored
 * `BackgroundInbox` element (`components/assistant-ui/elements/background-
 * inbox.tsx`) for the composer header's "background processes" panel.
 *
 * Three kinds of background work feed this panel:
 * - Detached (`mode: 'async'`) sub-agents spawned in this thread — the
 *   `BackgroundInbox`'s main `runs` list (via {@link selectBackgroundProcesses}).
 * - Scheduled (cron) jobs — rendered through the vendored `Timeline` element
 *   rather than hand-rolled rows, per the WS-D2 brief.
 * - Memory sync/ingestion status — rendered through the vendored
 *   `JobProgress` element, same reasoning.
 *
 * Collecting a settled run (`onCollect`) opens the whole-run Agent Process
 * Source panel scoped to that task's step — the same wiring
 * `BackgroundProcessesPanel`'s `onOpenProcess` used, now threaded through
 * `TranscriptOverlays`.
 */
import {
  BackgroundInbox,
  type BackgroundRun,
  type BackgroundState,
} from '../../../components/assistant-ui/elements/background-inbox';
import { JobProgress, type JobStage } from '../../../components/assistant-ui/elements/job-progress';
import { Timeline, type TimelineEvent } from '../../../components/assistant-ui/elements/timeline';
import { formatElapsed } from '../../../components/assistant-ui/utils/task';
import Button from '../../../components/ui/Button';
import { SheetContent, SheetRoot, SheetTitle } from '../../../components/ui/Sheet';
import { useT } from '../../../lib/i18n/I18nContext';
import type { CoreCronJob } from '../../../utils/tauriCommands/cron';
import { type MemorySyncSummary, useBackgroundActivity } from '../hooks/useBackgroundActivity';
import type { BackgroundProcess } from '../selectors/backgroundProcesses';
import { formatRelativeTime, formatResetTime } from '../utils/format';

function stateOf(status: BackgroundProcess['status']): BackgroundState {
  if (status === 'running' || status === 'awaiting_user') return 'running';
  if (status === 'error') return 'failed';
  return 'ready';
}

/** {@link BackgroundProcess} -> the vendored `BackgroundInbox`'s row shape. */
function toRun(process: BackgroundProcess): BackgroundRun {
  return {
    id: process.taskId,
    title: process.name,
    state: stateOf(process.status),
    elapsed:
      typeof process.elapsedMs === 'number'
        ? formatElapsed(process.elapsedMs)
        : typeof process.iterations === 'number'
          ? `${process.iterations}`
          : `${process.toolCount}`,
    summary: process.goal || undefined,
  };
}

/** Cron jobs, newest/soonest-relevant first, onto the vendored `Timeline`'s event shape. */
function cronJobsToEvents(jobs: CoreCronJob[]): TimelineEvent[] {
  return jobs.map(job => {
    const name = (job.name && job.name.trim()) || (job.prompt && job.prompt.trim()) || job.id;
    const when: TimelineEvent['when'] = !job.enabled ? 'future' : 'now';
    const time = job.enabled
      ? job.next_run
        ? formatResetTime(job.next_run)
        : ''
      : job.last_run
        ? formatRelativeTime(job.last_run)
        : '';
    return { id: job.id, when, time, title: name, detail: job.command ?? undefined };
  });
}

/** Memory sync/ingestion summary onto the vendored `JobProgress`'s stage shape. */
function memoryToJobProgress(memory: MemorySyncSummary): {
  stages: JobStage[];
  stageIndex: number;
  stageProgress: number;
  eta: string;
} {
  const stages: JobStage[] =
    memory.providers.length > 0
      ? memory.providers.map(row => ({ name: row.provider, weight: 1 }))
      : [{ name: 'memory', weight: 1 }];
  const stageIndex = memory.providers.filter(row => row.freshness !== 'active').length;
  const stageProgress = memory.ingesting ? 0.5 : stageIndex >= stages.length ? 1 : 0;
  const eta = memory.ingesting ? `${memory.queueDepth}` : '';
  return { stages, stageIndex, stageProgress, eta };
}

export interface BackgroundInboxCardProps {
  open: boolean;
  processes: BackgroundProcess[];
  onClose: () => void;
  /** Opens the Agent Process Source panel scoped to that task's step. */
  onOpenProcess: (taskId: string) => void;
}

export function BackgroundInboxCard({
  open,
  processes,
  onClose,
  onOpenProcess,
}: BackgroundInboxCardProps) {
  const { t } = useT();
  const activity = useBackgroundActivity(open);

  if (!open) return null;

  const runs = processes.map(toRun);
  const cronEvents = cronJobsToEvents(activity.cronJobs);
  const memoryHasActivity =
    activity.memory.ingesting ||
    activity.memory.queueDepth > 0 ||
    activity.memory.providers.length > 0;
  const memoryJob = memoryToJobProgress(activity.memory);

  return (
    <SheetRoot
      open
      onOpenChange={next => {
        if (!next) onClose();
      }}>
      <SheetContent
        side="right"
        aria-describedby={undefined}
        data-testid="background-processes-panel"
        className="">
        <header className="flex shrink-0 items-center justify-between border-b border-line-subtle px-4 py-3">
          <SheetTitle asChild>
            <h2 className="text-sm font-semibold text-content">
              {t('conversations.backgroundTasks.title')}
            </h2>
          </SheetTitle>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            aria-label={t('conversations.backgroundTasks.close')}
            onClick={onClose}>
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </Button>
        </header>

        <div className="flex-1 space-y-4 overflow-y-auto p-3">
          <BackgroundInbox
            runs={runs}
            onCollect={taskId => {
              const process = processes.find(p => p.taskId === taskId);
              if (process && stateOf(process.status) !== 'running') onOpenProcess(taskId);
            }}
            strings={{
              title: t('conversations.backgroundTasks.sectionThisChat'),
              ready: count =>
                t('conversations.backgroundTasks.inboxReady').replace('{count}', String(count)),
              inFlight: count =>
                t('conversations.backgroundTasks.inboxInFlight').replace('{count}', String(count)),
            }}
          />

          <div>
            <p className="mb-1.5 px-1 text-[11px] font-semibold uppercase tracking-wide text-content-faint">
              {t('conversations.backgroundTasks.sectionScheduled')}
            </p>
            {cronEvents.length > 0 ? (
              <Timeline events={cronEvents} visibleCount={cronEvents.length} />
            ) : (
              <p className="px-1 text-[12px] text-content-faint">
                {t('conversations.backgroundTasks.cronEmpty')}
              </p>
            )}
          </div>

          <div>
            <p className="mb-1.5 px-1 text-[11px] font-semibold uppercase tracking-wide text-content-faint">
              {t('conversations.backgroundTasks.sectionMemory')}
            </p>
            {memoryHasActivity ? (
              <JobProgress
                title={t('conversations.backgroundTasks.memoryJobTitle')}
                stages={memoryJob.stages}
                stageIndex={memoryJob.stageIndex}
                stageProgress={memoryJob.stageProgress}
                eta={memoryJob.eta}
                cancelLabel={t('conversations.backgroundTasks.cancelJob')}
              />
            ) : (
              <p className="px-1 text-[12px] text-content-faint">
                {t('conversations.backgroundTasks.memUpToDate')}
              </p>
            )}
          </div>
        </div>
      </SheetContent>
    </SheetRoot>
  );
}

export default BackgroundInboxCard;
