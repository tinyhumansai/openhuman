import createDebug from 'debug';

import Badge, { type BadgeVariant } from '../../../components/ui/Badge';
import Button from '../../../components/ui/Button';
import { SheetContent, SheetRoot, SheetTitle } from '../../../components/ui/Sheet';
import { useT } from '../../../lib/i18n/I18nContext';
import type {
  SubagentActivity,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';
import { useBackgroundActivity } from '../hooks/useBackgroundActivity';
import { CronJobRow, MemorySection, SectionHeader } from './BackgroundActivityRows';

const log = createDebug('app:conversations:background-processes');

/**
 * A background process = a *detached* sub-agent spawned with
 * `spawn_async_subagent` (a fire-and-forget tokio task that keeps running after
 * the parent turn returns). The backend marks these with `mode: "async"` on the
 * `SubagentSpawned` event (every blocking spawn emits `mode: "typed"`), and the
 * frontend carries it through on {@link SubagentActivity.mode}. So the whole
 * "is this truly in the background?" question reduces to `mode === 'async'`.
 */
export interface BackgroundProcess {
  taskId: string;
  name: string;
  goal: string;
  status: ToolTimelineEntryStatus;
  toolCount: number;
  iterations?: number;
}

const subagentName = (s: SubagentActivity): string =>
  (s.displayName && s.displayName.trim()) || s.agentId || 'sub-agent';

/**
 * Pure selector: the detached background sub-agents spawned in a thread,
 * newest-relevant first, deduped by spawn `taskId`. Driven off the same tool
 * timeline the inline rows and the {@link SubagentDrawer} use, so a process
 * opened here resolves to the exact same drawer entry.
 */
export function selectBackgroundProcesses(timeline: ToolTimelineEntry[]): BackgroundProcess[] {
  const seen = new Set<string>();
  const out: BackgroundProcess[] = [];
  for (const entry of timeline) {
    const sub = entry.subagent;
    if (!sub || sub.mode !== 'async') continue;
    if (seen.has(sub.taskId)) continue;
    seen.add(sub.taskId);
    out.push({
      taskId: sub.taskId,
      name: subagentName(sub),
      goal: (sub.prompt ?? '').trim(),
      status: entry.status,
      toolCount: sub.toolCalls?.length ?? 0,
      iterations: sub.iterations,
    });
  }
  // Running first, so live work stays at the top of the list.
  return out.sort((a, b) => Number(b.status === 'running') - Number(a.status === 'running'));
}

type StatusLabelKey =
  | 'conversations.backgroundTasks.statusRunning'
  | 'conversations.backgroundTasks.statusDone'
  | 'conversations.backgroundTasks.statusFailed'
  | 'conversations.backgroundTasks.statusNeedsYou'
  | 'conversations.backgroundTasks.statusCancelled';

/**
 * Dot fill + label key + shared {@link Badge} tone for one process status.
 *
 * The `error` / `awaiting_user` cases used raw Tailwind palette scales
 * (`bg-red-500`, `text-blue-700`) that do not follow a user's theme; they are
 * the semantic `coral` / `primary` tokens now, which is what every other
 * status surface in the conversation panel already used.
 */
function statusStyle(status: ToolTimelineEntryStatus): {
  dot: string;
  labelKey: StatusLabelKey;
  variant: BadgeVariant;
} {
  switch (status) {
    case 'running':
      return {
        dot: 'bg-amber-500 animate-pulse',
        labelKey: 'conversations.backgroundTasks.statusRunning',
        variant: 'warning',
      };
    case 'error':
      return {
        dot: 'bg-coral-500',
        labelKey: 'conversations.backgroundTasks.statusFailed',
        variant: 'danger',
      };
    case 'awaiting_user':
      return {
        dot: 'bg-primary-500',
        labelKey: 'conversations.backgroundTasks.statusNeedsYou',
        variant: 'primary',
      };
    case 'cancelled':
      return {
        dot: 'bg-content-faint',
        labelKey: 'conversations.backgroundTasks.statusCancelled',
        variant: 'neutral',
      };
    default:
      return {
        dot: 'bg-sage-500',
        labelKey: 'conversations.backgroundTasks.statusDone',
        variant: 'success',
      };
  }
}

interface BackgroundProcessesPanelProps {
  open: boolean;
  processes: BackgroundProcess[];
  onClose: () => void;
  onOpenProcess: (taskId: string) => void;
}

/**
 * Right side-drawer listing the thread's detached background sub-agents. Each
 * row opens the existing {@link SubagentDrawer} (via `onOpenProcess`) for the
 * full live transcript — this panel is purely the launcher/overview.
 */
export function BackgroundProcessesPanel({
  open,
  processes,
  onClose,
  onOpenProcess,
}: BackgroundProcessesPanelProps) {
  const { t } = useT();
  // Cron jobs + memory syncing — fetched only while open.
  const activity = useBackgroundActivity(open);

  if (!open) return null;

  const running = processes.filter(p => p.status === 'running').length;
  const runningLabel =
    running > 0
      ? t('conversations.backgroundTasks.running').replace('{count}', String(running))
      : t('conversations.backgroundTasks.noneRunning');
  const totalLabel = t('conversations.backgroundTasks.total').replace(
    '{count}',
    String(processes.length)
  );

  log(
    'render panel processes=%d running=%d cron=%d',
    processes.length,
    running,
    activity.cronJobs.length
  );

  // The overlay is the shared Radix-backed `Sheet`: the hand-rolled portal +
  // backdrop `<div onClick>` + `keydown` listener it replaced had no focus
  // trap, no scroll lock, no focus restore, and a backdrop that was not
  // keyboard-reachable at all. `open` is hard-coded because the early return
  // above already renders nothing when closed — `onOpenChange` routes Escape /
  // outside-click back to the caller's `onClose`.
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
        className="max-w-sm">
        <header className="flex shrink-0 items-center justify-between border-b border-line-subtle px-4 py-3">
          <div className="flex flex-col">
            {/* `asChild` keeps the historical h2 so the heading role and its
                accessible name are unchanged. */}
            <SheetTitle asChild>
              <h2 className="text-sm font-semibold text-content">
                {t('conversations.backgroundTasks.title')}
              </h2>
            </SheetTitle>
            <span className="text-[11px] text-content-faint">
              {runningLabel} · {totalLabel}
            </span>
          </div>
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

        <div className="flex-1 overflow-y-auto p-2">
          {/* Section 1 — detached sub-agents spawned in this chat. */}
          <SectionHeader title={t('conversations.backgroundTasks.sectionThisChat')} />
          {processes.length === 0 ? (
            <div className="px-2.5 py-2 text-[12px] text-content-faint">
              {t('conversations.backgroundTasks.empty')}
            </div>
          ) : (
            processes.map(p => {
              const s = statusStyle(p.status);
              const toolCallLabel = (
                p.toolCount === 1
                  ? t('conversations.backgroundTasks.toolCallOne')
                  : t('conversations.backgroundTasks.toolCallOther')
              ).replace('{count}', String(p.toolCount));
              return (
                <button
                  key={p.taskId}
                  type="button"
                  data-testid="background-process-row"
                  onClick={() => onOpenProcess(p.taskId)}
                  className="mb-1 flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-surface-hover">
                  <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${s.dot}`} />
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center justify-between gap-2">
                      <span className="truncate text-sm font-medium text-content">{p.name}</span>
                      <Badge variant={s.variant} className="shrink-0 rounded-full">
                        {t(s.labelKey)}
                      </Badge>
                    </span>
                    {p.goal ? (
                      <span className="mt-0.5 line-clamp-2 block text-[12px] text-content-muted">
                        {p.goal}
                      </span>
                    ) : null}
                    <span className="mt-0.5 block text-[11px] text-content-faint">
                      {toolCallLabel}
                      {typeof p.iterations === 'number'
                        ? ` · ${t('conversations.backgroundTasks.steps').replace('{count}', String(p.iterations))}`
                        : ''}{' '}
                      · {t('conversations.backgroundTasks.viewDetails')}
                    </span>
                  </span>
                </button>
              );
            })
          )}

          {/* Section 2 — scheduled (cron) jobs, global, view-only. */}
          <SectionHeader title={t('conversations.backgroundTasks.sectionScheduled')} />
          {activity.cronJobs.length === 0 ? (
            <div className="px-2.5 py-2 text-[12px] text-content-faint">
              {t('conversations.backgroundTasks.cronEmpty')}
            </div>
          ) : (
            activity.cronJobs.map(job => <CronJobRow key={job.id} job={job} />)
          )}

          {/* Section 4 — memory syncing / ingestion. */}
          <SectionHeader title={t('conversations.backgroundTasks.sectionMemory')} />
          <MemorySection memory={activity.memory} />
        </div>
      </SheetContent>
    </SheetRoot>
  );
}
