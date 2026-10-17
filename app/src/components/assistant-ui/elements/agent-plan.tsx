'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { useT } from '@/lib/i18n/I18nContext';
import { CheckIcon, Loader2Icon, MinusIcon, XIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { announced, pct, progressOf } from '../utils/range';
import { mono } from './surfaces';

export type AgentPlanStep = {
  id?: string | undefined;
  label: string;
  description?: string | undefined;
};

export function AgentPlan({
  steps,
  activeIndex,
  title = 'Plan',
  className,
  statuses,
  stepTestId,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'steps' | 'activeIndex'> & {
  steps: readonly (string | AgentPlanStep)[];
  activeIndex: number;
  title?: string | undefined;
  /** Core progress can complete steps out of order or stop on a failed step. */
  statuses?: readonly ('pending' | 'active' | 'done' | 'failed' | 'skipped')[];
  stepTestId?: string;
}) {
  const { t } = useT();
  const total = steps.length;
  const completed = statuses
    ? statuses.filter(status => status === 'done' || status === 'skipped').length
    : progressOf(activeIndex, total);
  const countLabel = t('chat.todos.ofTotal')
    .replace('{done}', String(completed))
    .replace('{total}', String(total));
  const allDone = completed >= total;
  const progress = pct(completed, total);

  return (
    <div data-slot="agent-plan" className={cn('flex w-full flex-col gap-3', className)} {...props}>
      <div className="flex items-center justify-between">
        <span className="text-[13.5px] font-medium">{title}</span>
        <span className={cn(mono, 'text-muted-foreground tabular-nums')}>{countLabel}</span>
      </div>
      <div
        role="progressbar"
        aria-label={title}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={announced(progress)}
        aria-valuetext={countLabel}
        className="bg-foreground/[0.06] inset-ring-border h-[3px] w-full overflow-hidden rounded-full inset-ring forced-colors:outline">
        <span
          aria-hidden
          className="bg-foreground/80 block h-full rounded-full transition-[width] duration-500 forced-color-adjust-none motion-reduce:transition-none"
          style={{ width: `${progress}%` }}
        />
      </div>
      <ul className="flex flex-col gap-2.5">
        {steps.map((step, i) => {
          const item = typeof step === 'string' ? { label: step } : step;
          const status =
            statuses?.[i] ??
            (allDone || i < completed ? 'done' : i === completed ? 'active' : 'pending');
          const done = status === 'done';
          const active = status === 'active';
          const failed = status === 'failed';
          const skipped = status === 'skipped';
          const statusText = t(
            status === 'done'
              ? 'conversations.taskCard.state.done'
              : skipped
                ? 'conversations.taskCard.state.cancelled'
                : status === 'failed'
                  ? 'conversations.taskCard.state.failed'
                  : status === 'active'
                    ? 'conversations.backgroundTasks.statusRunning'
                    : 'orchestration.runStatus.pending'
          );
          return (
            <li
              key={typeof step === 'string' ? i : (step.id ?? i)}
              data-testid={stepTestId}
              data-status={status}
              className={cn(
                'flex gap-2.5 text-[13.5px]',
                active && item.description ? 'items-start' : 'items-center'
              )}>
              <span className="flex size-4 shrink-0 items-center justify-center">
                {done ? (
                  <CheckIcon aria-hidden className="text-muted-foreground size-3.5" />
                ) : skipped ? (
                  <MinusIcon aria-hidden className="text-muted-foreground size-3.5" />
                ) : failed ? (
                  <XIcon aria-hidden className="text-destructive size-3.5" />
                ) : active ? (
                  <Loader2Icon
                    aria-hidden
                    className="text-foreground/90 size-3.5 animate-spin motion-reduce:animate-none"
                  />
                ) : (
                  <span
                    aria-hidden
                    className="bg-foreground/15 inset-ring-border size-1.5 rounded-full inset-ring forced-colors:border"
                  />
                )}
              </span>
              <span className="min-w-0">
                <span
                  className={cn(
                    done && 'text-muted-foreground',
                    active && 'text-foreground/90',
                    failed && 'text-destructive',
                    skipped && 'line-through decoration-[1.5px]',
                    !done && !active && !failed && 'text-muted-foreground'
                  )}>
                  {item.label}
                </span>
                <span className="sr-only">{` ${statusText}`}</span>
                {active && item.description ? (
                  <span className="text-muted-foreground mt-0.5 block text-xs">
                    {item.description}
                  </span>
                ) : null}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
