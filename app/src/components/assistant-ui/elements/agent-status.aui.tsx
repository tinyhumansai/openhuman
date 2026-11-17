'use client';

/**
 * Vendored from the assistant-ui `elements-agent-status` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-agent-status.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - `Popover`/`PopoverContent`/`PopoverTrigger` from
 *   `@/components/assistant-ui/ui/popover` (this app's Radix popover copy),
 *   not `@/components/ui/popover` (a differently-shaped shared component).
 * - `summaryLabel` takes an optional `strings` bag (English defaults
 *   matching upstream's hard-coded copy) so `AgentStatus`/`TaskTray` can be
 *   handed `useT()`-sourced copy from the host; every call site that omits
 *   it keeps upstream's exact English text.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/assistant-ui/ui/popover';
import { useT } from '@/lib/i18n/I18nContext';
import { type TaskState, useAuiState } from '@assistant-ui/react';
import { ChevronDownIcon } from 'lucide-react';
import { type FC, useMemo, useState } from 'react';

import {
  formatElapsed,
  TASK_PAGE_SIZE,
  taskLabel,
  taskMeta,
  taskStateOf,
  useTaskElapsed,
} from '../utils/task';
import { type AgentState, AgentStatus as AgentStatusBase } from './agent-status';
import { mono } from './surfaces';
import { TaskStateIcon } from './task-card';

export type TaskSummary = {
  readonly total: number;
  readonly running: number;
  readonly waiting: number;
  readonly failed: number;
  readonly startedAt: number | undefined;
  readonly runningLabel: string | undefined;
};

/** English defaults for {@link summaryLabel}; override via its `strings` param. */
export interface AgentStatusStrings {
  taskOne: string;
  taskOther: string;
  running: string;
  waitingForInput: string;
  done: string;
  failed: string;
  of: string;
}

const DEFAULT_STRINGS: AgentStatusStrings = {
  taskOne: 'task',
  taskOther: 'tasks',
  running: 'running',
  waitingForInput: 'waiting for input',
  done: 'done',
  failed: 'failed',
  of: 'of',
};

const summarize = (tasks: readonly TaskState[]): TaskSummary => {
  let running = 0;
  let waiting = 0;
  let failed = 0;
  let startedAt: number | undefined;
  let runningLabel: string | undefined;
  for (const task of tasks) {
    const state = taskStateOf(task.status, task.isError);
    if (state === 'working') {
      running += 1;
      runningLabel ??= taskLabel(task.toolName, task.args);
      const taskStartedAt = task.timing?.startedAt;
      if (taskStartedAt !== undefined && (startedAt === undefined || taskStartedAt < startedAt)) {
        startedAt = taskStartedAt;
      }
    } else if (state === 'waiting') {
      waiting += 1;
    } else if (state === 'failed') {
      failed += 1;
    }
  }
  return { total: tasks.length, running, waiting, failed, startedAt, runningLabel };
};

export const useTaskSummary = (): TaskSummary => {
  const tasks = useAuiState(s => s.thread.tasks);
  return useMemo(() => summarize(tasks), [tasks]);
};

const plural = (count: number, strings: AgentStatusStrings) =>
  `${count} ${count === 1 ? strings.taskOne : strings.taskOther}`;

const summaryState = (summary: TaskSummary): AgentState => {
  if (summary.running > 0) return 'working';
  if (summary.waiting > 0) return 'waiting';
  return summary.failed > 0 ? 'failed' : 'done';
};

const summaryLabel = (summary: TaskSummary, strings: AgentStatusStrings = DEFAULT_STRINGS) => {
  if (summary.running === 1 && summary.runningLabel !== undefined) {
    return summary.runningLabel;
  }
  if (summary.running > 0) {
    return `${summary.running} ${strings.of} ${plural(summary.total, strings)} ${strings.running}`;
  }
  if (summary.waiting > 0) {
    return `${plural(summary.waiting, strings)} ${strings.waitingForInput}`;
  }
  if (summary.failed > 0) {
    return `${plural(summary.total, strings)} ${strings.done}, ${summary.failed} ${strings.failed}`;
  }
  return `${plural(summary.total, strings)} ${strings.done}`;
};

const TaskTrayItem: FC<{ task: TaskState }> = ({ task }) => {
  const state = taskStateOf(task.status, task.isError);
  const meta = taskMeta(task.args);
  const elapsedMs = useTaskElapsed(
    task.timing,
    task.status.type === 'running' || task.status.type === 'requires-action'
  );

  return (
    <li
      data-slot="aui_task-tray-item"
      data-state={state}
      className="flex items-center gap-2.5 rounded-lg py-2 pe-2.5 text-[13px]"
      style={{ paddingInlineStart: `${0.625 + task.depth * 0.75}rem` }}>
      <TaskStateIcon state={state} />
      <span className="sr-only">{state}</span>
      <span className="min-w-0 flex-1 truncate">{taskLabel(task.toolName, task.args)}</span>
      {meta !== undefined && (
        <span className={cn(mono, 'text-muted-foreground max-w-24 shrink-0 truncate')}>{meta}</span>
      )}
      {elapsedMs !== undefined && (
        <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
          {formatElapsed(elapsedMs)}
        </span>
      )}
    </li>
  );
};

export const TaskTray: FC<{ className?: string; strings?: AgentStatusStrings }> = ({
  className,
  strings,
}) => {
  const { t } = useT();
  const tasks = useAuiState(s => s.thread.tasks);
  const summary = useMemo(() => summarize(tasks), [tasks]);
  const elapsedMs = useTaskElapsed(
    summary.startedAt === undefined ? undefined : { startedAt: summary.startedAt },
    summary.running > 0
  );
  const [open, setOpen] = useState(false);
  const [visible, setVisible] = useState(TASK_PAGE_SIZE);
  const firstTask = tasks[0];
  const listKey = firstTask === undefined ? '' : `${firstTask.messageId}:${firstTask.id}`;
  const [seenListKey, setSeenListKey] = useState(listKey);
  if (seenListKey !== listKey) {
    setSeenListKey(listKey);
    setVisible(TASK_PAGE_SIZE);
  }
  if (summary.total === 0 && open) setOpen(false);
  if (summary.total === 0) return null;
  const hidden = Math.max(0, tasks.length - visible);

  return (
    <Popover
      open={open}
      onOpenChange={next => {
        setOpen(next);
        if (!next) setVisible(TASK_PAGE_SIZE);
      }}>
      <PopoverTrigger
        className={cn(
          'focus-visible:ring-ring/50 cursor-pointer appearance-none rounded-full border-0 bg-transparent p-0 text-start focus-visible:ring-[3px] focus-visible:outline-none',
          className
        )}>
        <AgentStatusBase
          state={summaryState(summary)}
          label={summaryLabel(summary, strings)}
          elapsed={elapsedMs === undefined ? undefined : formatElapsed(elapsedMs)}
          trailing={
            <ChevronDownIcon
              className={cn(
                'size-3 transition-transform duration-200 motion-reduce:transition-none',
                open && 'rotate-180'
              )}
            />
          }
        />
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-1">
        <ul
          data-slot="aui_task-tray"
          aria-label={t('chat.filter.tasks', 'Tasks')}
          className="flex max-h-80 flex-col overflow-y-auto">
          {tasks.slice(0, visible).map((task, index) => (
            <TaskTrayItem key={`${index}:${task.id}`} task={task} />
          ))}
          {hidden > 0 && (
            <li className="flex">
              <button
                type="button"
                data-slot="aui_task-tray-more"
                onClick={() => setVisible(count => count + TASK_PAGE_SIZE)}
                className="text-muted-foreground hover:text-foreground px-2.5 py-2 text-xs transition-colors">
                {`+${Math.min(hidden, TASK_PAGE_SIZE)}`}
              </button>
            </li>
          )}
        </ul>
      </PopoverContent>
    </Popover>
  );
};
