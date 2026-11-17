'use client';

/**
 * assistant-ui's terminal-block element: a command and its output lines,
 * with a spinner while it runs and an exit marker once it is done.
 *
 * Vendored from assistant-ui `packages/ui/src/components/react/assistant-ui/elements/terminal-block.tsx`
 * (commit 1abca347). Changes from upstream:
 * - `exitLabel` / `failed`: upstream always reads "exit 0" with a check.
 * - The output scrolls past `max-h-72`, and the `min-h` floor applies only
 *   while output is still streaming (a settled one-line result should not
 *   reserve a tall box).
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CheckIcon, CircleXIcon, Loader2Icon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { take } from '../utils/range';
import { mono, paper } from './surfaces';

export function TerminalBlock({
  command,
  lines,
  visibleCount,
  done,
  failed = false,
  exitLabel = 'exit 0',
  variant = 'paper',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'command' | 'lines' | 'visibleCount' | 'done' | 'variant'
> & {
  command: string;
  lines: readonly string[];
  visibleCount: number;
  done: boolean;
  failed?: boolean;
  exitLabel?: string;
  variant?: 'paper' | 'ink';
}) {
  const ink = variant === 'ink';

  return (
    <div
      data-slot="terminal-block"
      className={cn(
        ink ? 'bg-foreground dark:bg-popover' : paper,
        'w-full overflow-hidden rounded-2xl font-mono text-xs',
        className
      )}
      {...props}>
      <div className="flex items-center justify-between gap-3 px-4 pt-3 pb-1.5">
        <span
          className={cn(
            'min-w-0 break-words',
            ink ? 'text-background/90 dark:text-foreground/90' : 'text-foreground/90'
          )}>
          {command}
        </span>
        {done ? (
          <div className="flex shrink-0 items-center gap-1">
            {failed ? (
              <CircleXIcon className="size-3 text-red-500" />
            ) : (
              <CheckIcon className="size-3 text-emerald-500" />
            )}
            <span
              className={cn(
                mono,
                ink ? 'text-background/40 dark:text-muted-foreground' : 'text-muted-foreground'
              )}>
              {exitLabel}
            </span>
          </div>
        ) : (
          <Loader2Icon
            className={cn(
              'size-3 animate-spin motion-reduce:animate-none',
              ink ? 'text-background/35 dark:text-muted-foreground' : 'text-muted-foreground'
            )}
          />
        )}
      </div>
      <div
        className={cn(
          'flex max-h-72 flex-col gap-1 overflow-auto px-4 pt-1 pb-3.5 whitespace-pre-wrap break-all',
          !done && 'min-h-[8.5rem]',
          ink ? 'text-background/55 dark:text-muted-foreground' : 'text-muted-foreground'
        )}>
        {take(lines, visibleCount).map((line, i) => {
          const isLast = i === lines.length - 1;
          return (
            <div
              key={`${i}-${line}`}
              className={cn(
                'fade-in animate-in fill-mode-both duration-300',
                isLast &&
                  (ink ? 'text-background/90 dark:text-foreground/90' : 'text-foreground/90')
              )}>
              {line}
            </div>
          );
        })}
        {!done && (
          <span
            aria-hidden
            className="inline-block h-3 w-1.5 animate-pulse bg-blue-500/70 motion-reduce:animate-none dark:bg-blue-400/70"
          />
        )}
      </div>
    </div>
  );
}
