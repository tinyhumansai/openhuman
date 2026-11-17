'use client';

/**
 * assistant-ui's web-search element: the query as a pill, a status line that
 * shimmers while searching, and the hits with a domain-initial avatar.
 *
 * Vendored from assistant-ui `packages/ui/src/components/react/assistant-ui/elements/web-search.tsx`
 * (commit 1abca347). Changes from upstream:
 * - The status line is props (`searchingLabel`, `statusLabel`); upstream
 *   hardcodes "Searching" / "Read 3 sources".
 * - A result may carry a `url`; the row then renders through `renderLink`
 *   so the host decides how an external link opens. A result's `url` is
 *   provider-supplied, so the host must only pass vetted http(s) URLs.
 * - Result keys include the URL: two hits often share a domain.
 * - No empty-results floor. Upstream reserves `min-h-[5.75rem]` while
 *   searching because its hits stream in one by one; here they arrive all
 *   at once with the settled result, so the floor was only a ~92px hole
 *   under every in-flight search (and a stack of them under parallel
 *   searches). The results list renders only when there is a hit.
 * - The query pill renders only once there is a query: while the call's
 *   arguments are still streaming it was an empty search-icon capsule.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { SearchIcon } from 'lucide-react';
import type { ComponentProps, ReactNode } from 'react';

import { take } from '../utils/range';
import { field, mono, ShimmerLabel } from './surfaces';

export interface WebSearchResult {
  title: string;
  domain: string;
  url?: string;
}

const rowClass =
  'fade-in slide-in-from-bottom-1 animate-in fill-mode-both hover:bg-foreground/[0.03] -mx-2.5 flex items-center gap-2.5 rounded-xl px-2.5 py-1.5 transition-colors duration-300';

export function WebSearch({
  query,
  results,
  visibleResults,
  searching,
  cycle,
  searchingLabel = 'Searching',
  statusLabel,
  renderLink,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'query' | 'results' | 'visibleResults' | 'searching' | 'cycle'
> & {
  query: string;
  results: readonly WebSearchResult[];
  visibleResults: number;
  searching: boolean;
  cycle: number;
  searchingLabel?: string;
  statusLabel: string;
  renderLink?: (props: { href: string; className: string; children: ReactNode }) => ReactNode;
}) {
  const shown = take(results, visibleResults);
  return (
    <div
      data-slot="web-search"
      className={cn('flex w-full flex-col gap-2.5', className)}
      {...props}>
      {query.trim() ? (
        <span
          data-slot="web-search-query"
          className={cn(
            field,
            'text-muted-foreground inline-flex w-fit items-center gap-1.5 rounded-full px-3.5 py-2 text-xs'
          )}>
          <SearchIcon className="text-muted-foreground size-3 shrink-0" />
          <span className="truncate">{query}</span>
        </span>
      ) : null}
      <div data-slot="web-search-status" className="text-muted-foreground text-xs">
        {searching ? (
          <ShimmerLabel className="relative inline-block leading-none">
            {searchingLabel}
          </ShimmerLabel>
        ) : (
          <span className="fade-in animate-in duration-300">{statusLabel}</span>
        )}
      </div>
      {shown.length > 0 ? (
        <div data-slot="web-search-results" className="flex min-h-[5.75rem] flex-col">
          {shown.map(result => {
            const content = (
              <>
                <span className="bg-foreground/[0.06] text-muted-foreground flex size-4 shrink-0 items-center justify-center rounded text-[9px] font-medium">
                  {result.domain.charAt(0).toUpperCase()}
                </span>
                <span className="text-foreground/90 min-w-0 flex-1 truncate text-[13.5px]">
                  {result.title}
                </span>
                <span className={cn(mono, 'text-muted-foreground shrink-0')}>{result.domain}</span>
              </>
            );
            const key = `${cycle}-${result.url ?? result.domain}-${result.title}`;
            return result.url && renderLink ? (
              <span key={key} data-slot="web-search-result" className="contents">
                {renderLink({ href: result.url, className: rowClass, children: content })}
              </span>
            ) : (
              <div key={key} data-slot="web-search-result" className={rowClass}>
                {content}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
