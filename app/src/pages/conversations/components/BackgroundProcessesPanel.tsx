import { useEffect } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type {
  SubagentActivity,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../store/chatRuntimeSlice';

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

function statusStyle(status: ToolTimelineEntryStatus): {
  dot: string;
  labelKey: StatusLabelKey;
  pill: string;
} {
  switch (status) {
    case 'running':
      return {
        dot: 'bg-amber-500 animate-pulse',
        labelKey: 'conversations.backgroundTasks.statusRunning',
        pill: 'text-amber-700 dark:text-amber-300',
      };
    case 'error':
      return {
        dot: 'bg-red-500',
        labelKey: 'conversations.backgroundTasks.statusFailed',
        pill: 'text-red-700 dark:text-red-300',
      };
    case 'awaiting_user':
      return {
        dot: 'bg-blue-500',
        labelKey: 'conversations.backgroundTasks.statusNeedsYou',
        pill: 'text-blue-700 dark:text-blue-300',
      };
    case 'cancelled':
      return {
        dot: 'bg-stone-400 dark:bg-neutral-500',
        labelKey: 'conversations.backgroundTasks.statusCancelled',
        pill: 'text-stone-600 dark:text-neutral-300',
      };
    default:
      return {
        dot: 'bg-sage-500',
        labelKey: 'conversations.backgroundTasks.statusDone',
        pill: 'text-sage-700 dark:text-sage-300',
      };
  }
}

export interface BackgroundProcessesPanelProps {
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

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

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

  return (
    <div className="fixed inset-0 z-50 flex justify-end" data-testid="background-processes-panel">
      <div className="absolute inset-0 bg-stone-900/30 dark:bg-black/50" onClick={onClose} />
      <aside className="relative flex h-full w-full max-w-sm flex-col bg-white dark:bg-neutral-900 shadow-xl">
        <header className="flex items-center justify-between border-b border-stone-100 px-4 py-3 dark:border-neutral-800">
          <div className="flex flex-col">
            <h2 className="text-sm font-semibold text-stone-800 dark:text-neutral-100">
              {t('conversations.backgroundTasks.title')}
            </h2>
            <span className="text-[11px] text-stone-400 dark:text-neutral-500">
              {runningLabel} · {totalLabel}
            </span>
          </div>
          <button
            type="button"
            aria-label={t('conversations.backgroundTasks.close')}
            onClick={onClose}
            className="flex h-7 w-7 items-center justify-center rounded-lg text-stone-400 hover:bg-stone-100 hover:text-stone-600 dark:text-neutral-500 dark:hover:bg-neutral-800 dark:hover:text-neutral-300">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </header>

        {processes.length === 0 ? (
          <div className="px-4 py-6 text-sm text-stone-400 dark:text-neutral-500">
            {t('conversations.backgroundTasks.empty')}
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto p-2">
            {processes.map(p => {
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
                  className="mb-1 flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-stone-100 dark:hover:bg-neutral-800">
                  <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${s.dot}`} />
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center justify-between gap-2">
                      <span className="truncate text-sm font-medium text-stone-800 dark:text-neutral-100">
                        {p.name}
                      </span>
                      <span className={`shrink-0 text-[11px] font-medium ${s.pill}`}>
                        {t(s.labelKey)}
                      </span>
                    </span>
                    {p.goal ? (
                      <span className="mt-0.5 line-clamp-2 block text-[12px] text-stone-500 dark:text-neutral-400">
                        {p.goal}
                      </span>
                    ) : null}
                    <span className="mt-0.5 block text-[11px] text-stone-400 dark:text-neutral-500">
                      {toolCallLabel}
                      {typeof p.iterations === 'number'
                        ? ` · ${t('conversations.backgroundTasks.steps').replace('{count}', String(p.iterations))}`
                        : ''}{' '}
                      · {t('conversations.backgroundTasks.viewDetails')}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </aside>
    </div>
  );
}
