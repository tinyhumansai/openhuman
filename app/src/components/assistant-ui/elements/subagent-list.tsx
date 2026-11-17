'use client';

/**
 * Vendored from the assistant-ui `elements-subagent-list` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-subagent-list.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - `../utils/range` -> `@/components/assistant-ui/utils/range`.
 * - `SubagentItem.done`: an agent's own finished flag. Upstream marks the
 *   first `completedCount` rows done, which is only right when workers finish
 *   in list order; parallel workers do not.
 * No hard-coded user-facing copy — `agent.name`/`agent.model` are
 * caller-supplied props, so there is nothing to route through `useT()` here.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { pct } from '@/components/assistant-ui/utils/range';
import { CheckIcon, Loader2Icon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { mono, paper } from './surfaces';

export interface SubagentItem {
  name: string;
  model: string;
  /** This agent has finished; falls back to `index < completedCount` when absent. */
  done?: boolean;
}

export function SubagentList({
  agents,
  completedCount,
  progress,
  showSummary,
  summaryAgent,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'agents' | 'completedCount' | 'progress' | 'showSummary' | 'summaryAgent'
> & {
  agents: readonly SubagentItem[];
  completedCount: number;
  progress: readonly number[];
  showSummary: boolean;
  summaryAgent: SubagentItem;
}) {
  return (
    <div
      data-slot="subagent-list"
      className={cn('flex w-full flex-col gap-2', className)}
      {...props}>
      {agents.map((agent, index) => {
        const done = agent.done ?? index < completedCount;
        const width = progress[index] ?? 0;
        const percentage = pct(width, 100);

        return (
          <div
            key={agent.name}
            className={cn(paper, 'flex flex-col gap-2 rounded-2xl px-3.5 py-2.5')}>
            <div className="flex items-center gap-2">
              {done ? (
                <CheckIcon className="fade-in zoom-in-90 animate-in size-3.5 shrink-0 text-emerald-500 duration-200" />
              ) : (
                <Loader2Icon className="text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
              )}
              <span className="flex-1 truncate text-[13.5px]">{agent.name}</span>
              <span className={cn(mono, 'text-muted-foreground')}>{agent.model}</span>
            </div>
            <span
              role="progressbar"
              aria-label={`${agent.name} progress`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percentage}
              className="bg-foreground/[0.06] inset-ring-border h-[3px] w-full overflow-hidden rounded-full inset-ring forced-colors:outline">
              <span
                className={cn(
                  'block h-full rounded-full transition-[width] duration-700 forced-color-adjust-none',
                  done ? 'bg-emerald-500/70' : 'bg-foreground/60'
                )}
                style={{ width: `${percentage}%` }}
              />
            </span>
          </div>
        );
      })}
      {showSummary && (
        <div
          className={cn(
            paper,
            'fade-in slide-in-from-bottom-2 animate-in flex flex-col gap-2 rounded-2xl px-3.5 py-2.5 duration-300'
          )}>
          <div className="flex items-center gap-2">
            <Loader2Icon className="text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
            <span className="flex-1 truncate text-[13.5px]">{summaryAgent.name}</span>
            <span className={cn(mono, 'text-muted-foreground')}>{summaryAgent.model}</span>
          </div>
          <span
            role="progressbar"
            aria-label={`${summaryAgent.name} progress`}
            aria-valuemin={0}
            aria-valuemax={100}
            className="bg-foreground/[0.06] inset-ring-border h-[3px] w-full overflow-hidden rounded-full inset-ring forced-colors:outline">
            <span className="shimmer shimmer-bg block h-full w-full rounded-full motion-reduce:animate-none" />
          </span>
        </div>
      )}
    </div>
  );
}
