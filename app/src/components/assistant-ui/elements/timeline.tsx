'use client';

/**
 * Vendored from the assistant-ui `elements-timeline` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-timeline.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - `../utils/range` -> this app's `@/components/assistant-ui/utils/range`
 *   (already vendored from the `elements-range` registry item).
 *
 * Note: also considered for WS-D's sub-agent activity feed; check this file
 * before adding a second copy — it renders any `TimelineEvent[]`, so both
 * surfaces (the conversation-map outline here, a sub-agent activity feed
 * there) can share it.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { take } from '@/components/assistant-ui/utils/range';
import type { ComponentProps } from 'react';

import { mono, paper } from './surfaces';

export type TimelineWhen = 'past' | 'now' | 'future';

export interface TimelineEvent {
  id: string;
  when: TimelineWhen;
  time: string;
  title: string;
  detail?: string;
}

export function Timeline({
  events,
  visibleCount,
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'events' | 'visibleCount'> & {
  events: readonly TimelineEvent[];
  visibleCount: number;
}) {
  return (
    <div
      data-slot="timeline"
      className={cn(paper, 'flex w-full flex-col rounded-2xl p-4', className)}
      {...props}>
      {take(events, visibleCount).map((event, i, shown) => (
        <div
          key={event.id}
          className="fade-in slide-in-from-left-1 animate-in fill-mode-both grid grid-cols-[3.5rem_1rem_minmax(0,1fr)] gap-x-2 duration-300">
          <span
            className={cn(
              mono,
              'pt-[3px] text-end tabular-nums',
              event.when === 'future' ? 'text-muted-foreground' : 'text-muted-foreground'
            )}>
            {event.time}
          </span>

          <span className="flex flex-col items-center">
            <span
              className={cn(
                'mt-1 size-2 shrink-0 rounded-full',
                event.when === 'now' &&
                  'bg-blue-500 ring-4 ring-blue-500/15 dark:bg-blue-400 forced-colors:bg-[Highlight]',
                event.when === 'past' && 'bg-muted-foreground forced-colors:bg-[CanvasText]',
                event.when === 'future' && 'border-muted-foreground border bg-transparent'
              )}
            />
            {i < shown.length - 1 && (
              <span
                className={cn(
                  'w-px flex-1',
                  event.when === 'future' ? 'bg-foreground/[0.08]' : 'bg-foreground/15'
                )}
              />
            )}
          </span>

          <div className={cn('flex flex-col gap-0.5', i < shown.length - 1 && 'pb-3')}>
            <span
              className={cn(
                'text-[13px] break-words',
                event.when === 'future' ? 'text-muted-foreground' : 'text-foreground/90',
                event.when === 'now' && 'font-medium'
              )}>
              {event.title}
            </span>
            {event.detail && (
              <span className="text-muted-foreground text-xs leading-relaxed break-words">
                {event.detail}
              </span>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}
