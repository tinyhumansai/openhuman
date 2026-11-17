'use client';

/**
 * The socket drops, the run keeps going on the server, and the stream is
 * picked back up.
 *
 * Vendored from the assistant-ui `elements-connection-state` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-connection-state.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - The "Connection lost…", "Reconnect", "Reconnecting", "Picked the stream
 *   back up.", "attempt N" and "+N tokens" captions are props with English
 *   defaults, for `useT()` — see `ConnectionStateBanner` in
 *   `features/conversations/aui/ConnectionStateBanner.tsx`, the only caller.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { CheckIcon, CloudOffIcon, Loader2Icon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { mono, paper } from './surfaces';

export type ConnectionPhase = 'online' | 'dropped' | 'reconnecting' | 'resumed';

export function ConnectionState({
  phase,
  attempt,
  resumedTokens,
  onRetry,
  droppedLabel = 'Connection lost. The run kept going on the server.',
  retryLabel = 'Reconnect',
  reconnectingLabel = 'Reconnecting',
  attemptLabel = (attempt: number) => `attempt ${attempt}`,
  resumedLabel = 'Picked the stream back up.',
  resumedTokensLabel = (tokens: number) => `+${tokens} tokens`,
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'phase' | 'attempt' | 'resumedTokens' | 'onRetry'> & {
  phase: ConnectionPhase;
  attempt?: number;
  resumedTokens?: number;
  onRetry?: () => void;
  droppedLabel?: string;
  retryLabel?: string;
  reconnectingLabel?: string;
  attemptLabel?: (attempt: number) => string;
  resumedLabel?: string;
  resumedTokensLabel?: (tokens: number) => string;
}) {
  if (phase === 'online') return null;

  return (
    <div
      data-slot="connection-state"
      className={cn(
        paper,
        'fade-in slide-in-from-top-1 animate-in flex w-full items-center gap-2.5 rounded-2xl px-3.5 py-2.5 duration-300',
        className
      )}
      {...props}>
      {phase === 'dropped' && (
        <>
          <CloudOffIcon className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
          <span className="min-w-0 flex-1 text-[13px]">{droppedLabel}</span>
          <button
            type="button"
            onClick={onRetry}
            className="text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/95 shrink-0 rounded-full px-2.5 py-1 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]">
            {retryLabel}
          </button>
        </>
      )}

      {phase === 'reconnecting' && (
        <>
          <Loader2Icon className="text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
          <span className="min-w-0 flex-1 text-[13px]">{reconnectingLabel}</span>
          {attempt !== undefined && (
            <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
              {attemptLabel(attempt)}
            </span>
          )}
        </>
      )}

      {phase === 'resumed' && (
        <>
          <CheckIcon className="size-3.5 shrink-0 text-emerald-500" />
          <span className="min-w-0 flex-1 text-[13px]">{resumedLabel}</span>
          {resumedTokens !== undefined && (
            <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
              {resumedTokensLabel(resumedTokens)}
            </span>
          )}
        </>
      )}
    </div>
  );
}
