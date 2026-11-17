'use client';

/**
 * Vendored from the assistant-ui `elements-error-state` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-error-state.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - "Retrying"/"Retry" are `retryingLabel`/`retryLabel` props with English
 *   defaults, for `useT()`.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CircleAlertIcon, RefreshCwIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { ShimmerLabel } from './surfaces';

export interface ErrorStateProps extends Omit<ComponentProps<'div'>, 'children' | 'role'> {
  title: string;
  detail: string;
  retrying: boolean;
  onRetry?: () => void;
  retryingLabel?: string;
  retryLabel?: string;
}

export function ErrorState({
  title,
  detail,
  retrying,
  onRetry,
  retryingLabel = 'Retrying',
  retryLabel = 'Retry',
  className,
  ...props
}: ErrorStateProps) {
  if (retrying) {
    return (
      <div
        data-slot="error-state"
        key="retrying"
        role="status"
        className={cn(
          'fade-in animate-in flex w-full items-center gap-2.5 text-sm duration-300 motion-reduce:animate-none',
          className
        )}
        {...props}>
        <RefreshCwIcon className="text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
        <ShimmerLabel className="text-muted-foreground relative inline-block">
          {retryingLabel}
        </ShimmerLabel>
      </div>
    );
  }

  return (
    <div
      data-slot="error-state"
      key="error"
      role="alert"
      className={cn(
        'fade-in animate-in flex w-full items-start gap-2.5 rounded-2xl bg-red-500/[0.06] px-4 py-3 text-sm duration-300 motion-reduce:animate-none dark:bg-red-500/10',
        className
      )}
      {...props}>
      <CircleAlertIcon className="mt-0.5 size-4 shrink-0 text-red-500/80" />
      <div>
        <p className="font-medium text-red-600 dark:text-red-400">{title}</p>
        <p className="mt-0.5 text-[13px] leading-snug text-red-600/60 dark:text-red-400/60">
          {detail}
        </p>
      </div>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="ms-auto flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium text-red-600 transition-colors hover:bg-red-500/10 dark:text-red-400">
          <RefreshCwIcon className="size-3" />
          {retryLabel}
        </button>
      )}
    </div>
  );
}
