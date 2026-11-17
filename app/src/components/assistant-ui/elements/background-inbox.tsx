'use client';

/**
 * Vendored from the assistant-ui `elements-background-inbox` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-background-inbox.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - The hard-coded "Running elsewhere" / "{n} ready" / "{n} in flight" copy
 *   is now a `strings` prop (English defaults matching upstream) so the host
 *   can supply `useT()`-sourced copy — see `aui/BackgroundInboxCard.tsx`.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CheckIcon, Loader2Icon, XIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { mono, paper } from './surfaces';

export type BackgroundState = 'running' | 'ready' | 'failed';

export interface BackgroundRun {
  id: string;
  title: string;
  state: BackgroundState;
  elapsed: string;
  summary?: string;
}

/** English defaults for the inbox header; override via the `strings` prop. */
export interface BackgroundInboxStrings {
  title: string;
  ready: (count: number) => string;
  inFlight: (count: number) => string;
}

const DEFAULT_STRINGS: BackgroundInboxStrings = {
  title: 'Running elsewhere',
  ready: count => `${count} ready`,
  inFlight: count => `${count} in flight`,
};

export function BackgroundInbox({
  runs,
  onCollect,
  strings = DEFAULT_STRINGS,
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'runs' | 'onCollect'> & {
  runs: readonly BackgroundRun[];
  onCollect?: (id: string) => void;
  strings?: BackgroundInboxStrings;
}) {
  const ready = runs.filter(run => run.state === 'ready').length;
  const running = runs.filter(run => run.state === 'running').length;

  return (
    <div
      data-slot="background-inbox"
      className={cn(paper, 'flex w-full flex-col gap-1 rounded-2xl p-3', className)}
      {...props}>
      <div className="flex items-baseline justify-between px-1 pb-1">
        <span className="text-[13.5px] font-medium">{strings.title}</span>
        <span
          className={cn(
            mono,
            'tabular-nums',
            ready > 0 ? 'text-blue-600 dark:text-blue-400' : 'text-muted-foreground'
          )}>
          {ready > 0 ? strings.ready(ready) : strings.inFlight(running)}
        </span>
      </div>

      {runs.map(run => {
        const rowClassName = cn(
          'flex items-center gap-2.5 rounded-xl px-1.5 py-2 text-start transition-colors',
          run.state === 'running'
            ? 'cursor-default'
            : onCollect
              ? 'hover:bg-foreground/[0.04]'
              : undefined
        );
        const content = (
          <>
            <span className="flex size-3.5 shrink-0 items-center justify-center">
              {run.state === 'running' ? (
                <Loader2Icon className="text-muted-foreground size-3 animate-spin motion-reduce:animate-none" />
              ) : run.state === 'failed' ? (
                <XIcon className="size-3 text-red-500" />
              ) : (
                <CheckIcon className="size-3 text-emerald-500" />
              )}
            </span>

            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span
                className={cn(
                  'truncate text-[13px]',
                  run.state === 'running' ? 'text-muted-foreground' : 'text-foreground/90'
                )}>
                {run.title}
              </span>
              {run.summary && (
                <span className={cn(mono, 'text-muted-foreground truncate')}>{run.summary}</span>
              )}
            </span>

            <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
              {run.elapsed}
            </span>
          </>
        );

        return onCollect ? (
          <button
            key={run.id}
            type="button"
            disabled={run.state === 'running'}
            onClick={() => onCollect(run.id)}
            className={rowClassName}>
            {content}
          </button>
        ) : (
          <div key={run.id} className={rowClassName}>
            {content}
          </div>
        );
      })}
    </div>
  );
}
