'use client';

/**
 * Vendored from the assistant-ui `elements-conversation-search` registry
 * item (https://r.assistant-ui.com/styles/base-nova/elements-conversation-search.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - "Find in conversation" placeholder/aria-label and the previous/next match
 *   aria-labels are props with English defaults, for `useT()` — see
 *   `features/conversations/aui/ChatConversationSearch.tsx`.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { ChevronDownIcon, ChevronUpIcon, SearchIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, ghostButton, mono, paper } from './surfaces';

export interface SearchHit {
  id: string;
  before: string;
  match: string;
  after: string;
  position: number;
}

export function ConversationSearch({
  query,
  hits,
  activeIndex,
  onQueryChange,
  onStep,
  placeholder = 'Find in conversation',
  previousMatchLabel = 'Previous match',
  nextMatchLabel = 'Next match',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'query' | 'hits' | 'activeIndex' | 'onQueryChange' | 'onStep'
> & {
  query: string;
  hits: readonly SearchHit[];
  activeIndex: number;
  onQueryChange?: (query: string) => void;
  onStep?: (delta: number) => void;
  placeholder?: string;
  previousMatchLabel?: string;
  nextMatchLabel?: string;
}) {
  const index = hits.length === 0 ? -1 : Math.min(Math.max(activeIndex, 0), hits.length - 1);
  const active = index === -1 ? undefined : hits[index];

  return (
    <div data-slot="conversation-search" className={cn('flex w-full gap-2', className)} {...props}>
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className={cn(paper, 'flex items-center gap-2 rounded-full py-1.5 pr-1.5 pl-3')}>
          <SearchIcon className="text-muted-foreground size-3.5 shrink-0" />
          <input
            value={query}
            onChange={event => onQueryChange?.(event.target.value)}
            placeholder={placeholder}
            aria-label={placeholder}
            className="text-foreground/85 placeholder:text-muted-foreground min-w-0 flex-1 bg-transparent text-[13px] outline-none"
          />
          <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
            {hits.length === 0 ? '0' : `${index + 1}/${hits.length}`}
          </span>
          {onStep && (
            <>
              <button
                type="button"
                aria-label={previousMatchLabel}
                onClick={() => onStep(-1)}
                className={cn(ghostButton, 'size-6 shrink-0')}>
                <ChevronUpIcon className="size-3.5" />
              </button>
              <button
                type="button"
                aria-label={nextMatchLabel}
                onClick={() => onStep(1)}
                className={cn(ghostButton, 'size-6 shrink-0')}>
                <ChevronDownIcon className="size-3.5" />
              </button>
            </>
          )}
        </div>

        {active && (
          <div
            className={cn(
              field,
              'fade-in animate-in rounded-xl px-3 py-2 text-xs leading-relaxed duration-200'
            )}>
            <span className="text-muted-foreground">{active.before}</span>
            <span className="text-foreground/95 rounded bg-amber-400/35 px-0.5">
              {active.match}
            </span>
            <span className="text-muted-foreground">{active.after}</span>
          </div>
        )}
      </div>

      <div className="bg-foreground/[0.04] inset-ring-border relative w-1.5 shrink-0 rounded-full inset-ring forced-colors:border">
        {hits.map((hit, i) => (
          <span
            key={hit.id}
            aria-hidden
            className={cn(
              'absolute inset-x-0 h-1 rounded-full transition-colors duration-200 forced-color-adjust-none',
              i === index
                ? 'bg-amber-500 forced-colors:bg-[Highlight]'
                : 'bg-amber-500/35 forced-colors:bg-[CanvasText]'
            )}
            style={{ top: `${hit.position}%` }}
          />
        ))}
      </div>
    </div>
  );
}
