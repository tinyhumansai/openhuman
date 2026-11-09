'use client';

/**
 * A failed tool call: what broke, said in one line, with Retry/Skip.
 *
 * Vendored from the assistant-ui `elements-tool-error` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-tool-error.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - `name`/`target`/`message`/labels are props with English defaults, for
 *   `useT()` — see `ToolFailureCard` in
 *   `features/conversations/aui/ToolFailureCard.tsx`, the only caller.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { Button } from '@/components/ui';
import { AlertCircleIcon, Loader2Icon, RotateCwIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, mono, paper } from './surfaces';

export function ToolError({
  name,
  target,
  message,
  attempt,
  maxAttempts,
  retrying,
  onRetry,
  onSkip,
  retryLabel = 'Retry',
  retryingLabel = 'Retrying',
  skipLabel = 'Skip',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  | 'children'
  | 'name'
  | 'target'
  | 'message'
  | 'attempt'
  | 'maxAttempts'
  | 'retrying'
  | 'onRetry'
  | 'onSkip'
> & {
  name: string;
  target: string;
  message: string;
  attempt?: number;
  maxAttempts?: number;
  retrying: boolean;
  onRetry?: () => void;
  onSkip?: () => void;
  retryLabel?: string;
  retryingLabel?: string;
  skipLabel?: string;
}) {
  return (
    <div
      data-slot="tool-error"
      className={cn(paper, 'flex w-full flex-col gap-3 rounded-2xl p-3.5', className)}
      {...props}>
      <div className="flex items-center gap-2.5">
        <AlertCircleIcon className="size-3.5 shrink-0 text-red-500" />
        <span className={cn(mono, 'text-muted-foreground shrink-0')}>{name}</span>
        <span className="text-foreground/80 min-w-0 flex-1 truncate text-[13px]">{target}</span>
        {attempt !== undefined && maxAttempts !== undefined ? (
          <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
            {attempt}/{maxAttempts}
          </span>
        ) : null}
      </div>

      <div
        className={cn(
          field,
          'rounded-xl px-3 py-2 text-sm leading-relaxed break-words text-red-700 dark:text-red-300'
        )}>
        {message}
      </div>

      {onRetry || onSkip ? (
        <div className="flex items-center justify-end gap-2">
          {onSkip ? (
            <Button type="button" variant="tertiary" size="sm" onClick={onSkip}>
              {skipLabel}
            </Button>
          ) : null}
          {onRetry ? (
            <Button
              type="button"
              variant="tertiary"
              size="sm"
              onClick={onRetry}
              disabled={retrying}>
              {retrying ? (
                <Loader2Icon className="size-3 animate-spin motion-reduce:animate-none" />
              ) : (
                <RotateCwIcon className="size-3" />
              )}
              {retrying ? retryingLabel : retryLabel}
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
