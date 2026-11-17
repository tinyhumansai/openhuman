/**
 * Vendored from assistant-ui's Conversation Map registry element.
 * Source: https://r.assistant-ui.com/base/elements-conversation-map.json
 */
'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { useT } from '@/lib/i18n/I18nContext';
import { type ComponentProps, type KeyboardEvent, useCallback, useRef, useState } from 'react';

import { clamp } from '../utils/range';
import { floating } from './surfaces';

export interface ConversationMapEntry {
  id: string;
  title: string;
  preview?: string;
}

const TICK = '[data-slot="conversation-map-tick"]';

export function ConversationMap({
  entries,
  activeId,
  visibleIds,
  onSelect,
  side = 'right',
  align = 'left',
  className,
  onKeyDown,
  ...props
}: Omit<ComponentProps<'nav'>, 'children' | 'onSelect'> & {
  entries: readonly ConversationMapEntry[];
  activeId?: string | undefined;
  visibleIds?: readonly string[] | undefined;
  onSelect?: ((id: string) => void) | undefined;
  side?: 'left' | 'right';
  /** Which edge of the 24px rail the collapsed ticks hug. */
  align?: 'left' | 'right';
}) {
  const { t } = useT();
  const railRef = useRef<HTMLElement>(null);
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const [previewId, setPreviewId] = useState<string | null>(null);

  const inView = new Set(visibleIds);
  const activeIndex = entries.findIndex(entry => entry.id === activeId);
  const tabbableIndex = clamp(
    focusedIndex ?? Math.max(0, activeIndex),
    0,
    Math.max(0, entries.length - 1)
  );
  const previewEntry = entries.find(entry => entry.id === previewId);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLElement>) => {
      onKeyDown?.(event);
      if (event.defaultPrevented) return;

      const ticks = railRef.current?.querySelectorAll<HTMLElement>(TICK);
      if (!ticks?.length) return;

      const current = Array.prototype.indexOf.call(ticks, event.target);
      if (current === -1) return;

      const next = { ArrowUp: current - 1, ArrowDown: current + 1, Home: 0, End: ticks.length - 1 }[
        event.key
      ];
      if (next === undefined) return;

      event.preventDefault();
      ticks[clamp(next, 0, ticks.length - 1)]?.focus();
    },
    [onKeyDown]
  );

  return (
    <nav
      data-slot="conversation-map"
      ref={railRef}
      aria-label={t('elements.conversationMap.ariaLabel', 'Conversation map')}
      onKeyDown={handleKeyDown}
      onPointerLeave={() => setPreviewId(null)}
      className={cn('group/rail flex h-full w-6 flex-col justify-center', className)}
      {...props}>
      {entries.map((entry, index) => {
        const current = index === activeIndex;
        const onScreen = current || inView.has(entry.id);
        return (
          <button
            key={entry.id}
            type="button"
            data-slot="conversation-map-tick"
            data-active={current ? '' : undefined}
            data-in-view={onScreen ? '' : undefined}
            aria-label={entry.title}
            aria-current={current ? 'true' : undefined}
            tabIndex={index === tabbableIndex ? 0 : -1}
            onPointerEnter={() => setPreviewId(entry.id)}
            onFocus={() => {
              setFocusedIndex(index);
              setPreviewId(entry.id);
            }}
            onClick={onSelect ? () => onSelect(entry.id) : undefined}
            // The cap keeps a short thread packed instead of spread over the
            // whole gutter; a long one outgrows it and the share decides.
            className={cn(
              'group flex max-h-3.5 min-h-0 flex-1 items-center outline-none',
              align === 'right' ? 'justify-end' : 'justify-start'
            )}>
            {/* At rest every tick is the same short length and only weight and
                depth separate the tiers. Pointing at the rail grows them out by
                tier, and the one actually under the pointer reaches full length
                so the rail says which turn the card belongs to. */}
            <span
              className={cn(
                'w-3 rounded-full transition-[width,height,background-color] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none',
                current
                  ? 'bg-foreground/90 h-[3px] group-focus-within/rail:w-6 group-hover/rail:w-6'
                  : cn(
                      'group-hover:bg-foreground/70 group-focus-visible:bg-foreground/70 h-0.5',
                      'group-hover:w-6! group-focus-visible:w-6!',
                      onScreen
                        ? 'bg-foreground/50 group-focus-within/rail:w-[18px] group-hover/rail:w-[18px]'
                        : 'bg-foreground/15'
                    )
              )}
            />
          </button>
        );
      })}

      {previewEntry && (
        <div
          data-slot="conversation-map-preview"
          data-side={side}
          onPointerLeave={() => setPreviewId(null)}
          className={cn(
            floating,
            'absolute top-1/2 z-50 w-60 -translate-y-1/2 rounded-2xl bg-popover/95 p-3.5 shadow-xl ring-1 ring-foreground/15 backdrop-blur-xl outline-none',
            side === 'left' ? 'right-full me-2.5' : 'left-full ms-2.5'
          )}>
          <p className="line-clamp-2 text-[13px] leading-snug font-medium">{previewEntry.title}</p>
          {previewEntry.preview && (
            <p className="text-muted-foreground mt-1 line-clamp-3 text-[13px] leading-relaxed">
              {previewEntry.preview}
            </p>
          )}
        </div>
      )}
    </nav>
  );
}
