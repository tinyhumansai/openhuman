'use client';

/**
 * The running message and the messages queued behind it, each removable.
 *
 * Vendored from the assistant-ui `elements-message-queue` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-message-queue.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - The "running", "N queued" and "sends when this finishes" captions and the
 *   remove button's accessible name are props with English defaults, for
 *   `useT()` — see `ComposerMessageQueue` in
 *   `features/conversations/aui/ComposerMessageQueue.tsx`, the only caller.
 * - No decorative send arrow on a queued row: it read as a send-now button
 *   that does nothing, and the core queue has no send-now action (#7730).
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { XIcon } from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';

import { field, ghostButton, mono, paper } from './surfaces';

export interface QueuedMessage {
  id: string;
  text: string;
}

export function MessageQueue({
  running,
  queued = [],
  queuedCount = queued.length,
  children,
  onCancel,
  runningLabel = 'running',
  queuedLabel = (count: number) => `${count} queued`,
  pendingHint = 'sends when this finishes',
  removeLabel = (text: string) => `Remove "${text}" from the queue`,
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'running' | 'queued' | 'onCancel'> & {
  running: string;
  queued?: readonly QueuedMessage[];
  queuedCount?: number;
  /** Native runtime queue rows; omitted by standalone previews. */
  children?: ReactNode;
  onCancel?: (id: string) => void;
  runningLabel?: string;
  queuedLabel?: (count: number) => string;
  pendingHint?: string;
  removeLabel?: (text: string) => string;
}) {
  return (
    <div
      data-slot="message-queue"
      className={cn('flex w-full flex-col gap-2', className)}
      {...props}>
      <div className={cn(paper, 'flex items-center gap-2.5 rounded-2xl p-3')}>
        <span className="relative flex size-2 shrink-0">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-blue-500/60 motion-reduce:hidden" />
          <span className="relative inline-flex size-2 rounded-full bg-blue-500 dark:bg-blue-400" />
        </span>
        <span className="text-foreground/90 min-w-0 flex-1 truncate text-[13.5px]">{running}</span>
        <span className={cn(mono, 'text-muted-foreground shrink-0')}>{runningLabel}</span>
      </div>

      {queuedCount > 0 && (
        <div className="flex items-baseline justify-between px-1">
          <span className={cn(mono, 'text-muted-foreground')}>{queuedLabel(queuedCount)}</span>
          <span className={cn(mono, 'text-muted-foreground')}>{pendingHint}</span>
        </div>
      )}

      <ul className="flex flex-col gap-1.5">
        {children ??
          queued.map((message, index) => (
            <MessageQueueItem
              key={message.id}
              position={index + 1}
              text={message.text}
              action={
                onCancel ? (
                  <button
                    type="button"
                    aria-label={removeLabel(message.text)}
                    onClick={() => onCancel(message.id)}
                    className={cn(ghostButton, 'size-6 shrink-0')}>
                    <XIcon className="size-3.5" />
                  </button>
                ) : null
              }
            />
          ))}
      </ul>
    </div>
  );
}

/** Shared registry row; runtime text and actions are supplied by queue primitives. */
export function MessageQueueItem({
  position,
  text,
  action,
}: {
  position: number;
  text: ReactNode;
  action?: ReactNode;
}) {
  return (
    <li
      className={cn(
        field,
        'fade-in slide-in-from-bottom-1 animate-in fill-mode-both flex items-center gap-2.5 rounded-2xl py-2 pr-2 pl-3 duration-300'
      )}>
      <span className={cn(mono, 'text-muted-foreground w-3 shrink-0 tabular-nums')}>
        {position}
      </span>
      <span className="text-muted-foreground min-w-0 flex-1 truncate text-[13.5px]">{text}</span>
      {action}
    </li>
  );
}
