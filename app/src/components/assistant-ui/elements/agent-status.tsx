'use client';

/**
 * A compact status pill for a long-running agent activity: state dot/icon,
 * label, optional elapsed time, and a trailing slot (defaults to a
 * pause/retry icon).
 *
 * Vendored from the assistant-ui `elements-agent-status` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-agent-status.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * No literal user-facing strings here — every visible value already comes
 * from props/children, so there is nothing to route through `useT()`.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CheckIcon, PauseIcon, RotateCcwIcon, XIcon } from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';

import { mono, paper } from './surfaces';

export type AgentState = 'working' | 'waiting' | 'done' | 'failed';

export interface StatusStep {
  state: AgentState;
  label: string;
}

export function AgentStatus({
  state,
  label,
  elapsed,
  trailing,
  className,
  ...props
}: Omit<ComponentProps<'span'>, 'children' | 'state' | 'label' | 'elapsed'> & {
  state: AgentState;
  label: string;
  elapsed?: string | undefined;
  trailing?: ReactNode | undefined;
}) {
  return (
    <span
      data-slot="agent-status"
      className={cn(
        paper,
        'flex items-center gap-2.5 rounded-full py-1.5 ps-3.5 pe-1.5',
        className
      )}
      {...props}>
      {state === 'done' ? (
        <CheckIcon aria-hidden className="size-3 shrink-0 text-emerald-500" />
      ) : state === 'failed' ? (
        <XIcon aria-hidden className="text-destructive size-3 shrink-0" />
      ) : (
        <span
          aria-hidden
          className={cn(
            'size-1.5 shrink-0 rounded-full motion-reduce:animate-none',
            state === 'working'
              ? 'animate-pulse bg-blue-500 dark:bg-blue-400'
              : 'border-foreground/35 border'
          )}
        />
      )}
      <span className="sr-only">{state}</span>
      <span
        key={label}
        data-slot="agent-status-label"
        className="fade-in blur-in-[2px] animate-in max-w-44 truncate text-xs duration-300 motion-reduce:animate-none">
        {label}
      </span>
      {elapsed !== undefined && state !== 'done' && state !== 'failed' && (
        <span className={cn(mono, 'text-muted-foreground tabular-nums')}>{elapsed}</span>
      )}
      <span
        aria-hidden
        data-slot="agent-status-trailing"
        className="text-muted-foreground flex size-6 items-center justify-center rounded-full">
        {trailing !== undefined ? (
          trailing
        ) : state === 'done' || state === 'failed' ? (
          <RotateCcwIcon className="size-3" />
        ) : (
          <PauseIcon className="size-3" />
        )}
      </span>
    </span>
  );
}
