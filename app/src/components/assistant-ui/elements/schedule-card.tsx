'use client';

/**
 * Vendored from the assistant-ui `elements-schedule-card` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-schedule-card.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - The "next" / "paused" / "recent runs" / "ok" / "failed" labels and the
 *   `Pause {name}` / `Resume {name}` toggle aria-label are props with English
 *   defaults, for `useT()` — see `features/conversations/aui/ChatScheduleCard.tsx`.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CheckIcon, ClockIcon, XIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, mono, paper } from './surfaces';

export interface ScheduleRun {
  id: string;
  at: string;
  ok: boolean;
}

export function ScheduleCard({
  name,
  cadence,
  nextRun,
  enabled,
  history,
  onToggle,
  nextLabel = 'next',
  pausedLabel = 'paused',
  recentRunsLabel = 'recent runs',
  okLabel = 'ok',
  failedLabel = 'failed',
  toggleAriaLabel = (isEnabled: boolean, jobName: string) =>
    `${isEnabled ? 'Pause' : 'Resume'} ${jobName}`,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'name' | 'cadence' | 'nextRun' | 'enabled' | 'history' | 'onToggle'
> & {
  name: string;
  cadence: string;
  nextRun: string;
  enabled: boolean;
  history: readonly ScheduleRun[];
  onToggle?: () => void;
  nextLabel?: string;
  pausedLabel?: string;
  recentRunsLabel?: string;
  okLabel?: string;
  failedLabel?: string;
  toggleAriaLabel?: (enabled: boolean, name: string) => string;
}) {
  return (
    <div
      data-slot="schedule-card"
      className={cn(paper, 'flex w-full flex-col gap-3 rounded-2xl p-4', className)}
      {...props}>
      <div className="flex items-center gap-2.5">
        <span className="bg-foreground/[0.05] text-muted-foreground flex size-7 shrink-0 items-center justify-center rounded-lg">
          <ClockIcon className="size-3.5" />
        </span>
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-[13.5px] font-medium">{name}</span>
          <span className={cn(mono, 'text-muted-foreground')}>{cadence}</span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={enabled}
          aria-label={toggleAriaLabel(enabled, name)}
          onClick={onToggle}
          className={cn(
            'flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors duration-200',
            enabled ? 'bg-foreground/80' : 'bg-foreground/15'
          )}>
          <span
            className={cn(
              'bg-background size-4 rounded-full transition-transform duration-200 motion-reduce:transition-none',
              enabled && 'translate-x-4'
            )}
          />
        </button>
      </div>

      <div
        className={cn(
          field,
          'flex items-baseline gap-2 rounded-xl px-3 py-2',
          !enabled && 'opacity-45'
        )}>
        <span className={cn(mono, 'text-muted-foreground')}>{nextLabel}</span>
        <span className="text-foreground/80 text-[13px]">{enabled ? nextRun : pausedLabel}</span>
      </div>

      <div className="flex flex-col gap-1">
        <span className={cn(mono, 'text-muted-foreground')}>{recentRunsLabel}</span>
        {history.map(run => (
          <div key={run.id} className="flex items-baseline gap-2">
            {run.ok ? (
              <CheckIcon className="size-3 shrink-0 translate-y-0.5 text-emerald-500" />
            ) : (
              <XIcon className="size-3 shrink-0 translate-y-0.5 text-red-500" />
            )}
            <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs">{run.at}</span>
            <span className={cn(mono, 'text-muted-foreground shrink-0')}>
              {run.ok ? okLabel : failedLabel}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
