'use client';

/**
 * Where the context window goes: a stacked bar of labelled segments against
 * the limit, one row per segment and a headroom row.
 *
 * Vendored from the assistant-ui `elements-context-breakdown` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-context-breakdown.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - The "Context" title, the "Headroom" row, and each meter's accessible name
 *   and value text are props with English defaults, for `useT()` — see
 *   `ContextUsage` in `features/conversations/aui/ContextUsage.tsx`, the only
 *   caller.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import type { ComponentProps } from 'react';

import { announced, pct } from '../utils/range';
import { formatTokenCount } from './context-display';
import { mono, paper } from './surfaces';

const fmt = (n: number) => n.toLocaleString('en-US');

export interface ContextSegment {
  label: string;
  tokens: number;
  tint: string;
}

export interface ContextStat {
  label: string;
  value: string;
}

export function ContextBreakdown({
  segments,
  limit,
  title = 'Context',
  headroomLabel = 'Headroom',
  meterLabel = label => `${label} context usage`,
  meterValueText = (used, max) => `${used} of ${max}`,
  stats = [],
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'segments' | 'limit' | 'title'> & {
  segments: readonly ContextSegment[];
  limit: number;
  title?: string;
  headroomLabel?: string;
  meterLabel?: (label: string) => string;
  meterValueText?: (used: string, limit: string) => string;
  /** Informational rows that do not consume the parent model's context window. */
  stats?: readonly ContextStat[];
}) {
  const used = segments.reduce((sum, segment) => sum + segment.tokens, 0);
  const pressure = limit === 0 ? 0 : used / limit;
  const share = (tokens: number) => pct(tokens, limit);

  return (
    <div
      data-slot="context-breakdown"
      className={cn(paper, 'flex w-full flex-col gap-3 rounded-2xl p-4', className)}
      {...props}>
      <div className="flex items-baseline justify-between">
        <span className="text-[13.5px] font-medium">{title}</span>
        <span
          className={cn(
            mono,
            'tabular-nums',
            pressure > 0.85 ? 'text-amber-600 dark:text-amber-400' : 'text-muted-foreground'
          )}>
          {formatTokenCount(used)} / {formatTokenCount(limit)}
        </span>
      </div>

      <div className="bg-foreground/[0.06] inset-ring-border flex h-2 w-full overflow-hidden rounded-full inset-ring forced-colors:border">
        {segments.map(segment => {
          const width = share(segment.tokens);
          if (announced(width) === 0) return null;
          return (
            <span
              key={segment.label}
              role="meter"
              aria-label={meterLabel(segment.label)}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={announced(width)}
              aria-valuetext={meterValueText(fmt(segment.tokens), fmt(limit))}
              className={cn(
                'h-full transition-[width] duration-500 ease-out forced-color-adjust-none motion-reduce:transition-none',
                segment.tint
              )}
              style={{ width: `${width}%` }}
            />
          );
        })}
      </div>

      <div className="flex flex-col gap-1.5">
        {segments.map(segment => (
          <div key={segment.label} className="flex items-center gap-2">
            <span
              aria-hidden
              className={cn('size-2 shrink-0 rounded-full forced-color-adjust-none', segment.tint)}
            />
            <span className="text-muted-foreground min-w-0 flex-1 truncate text-[13px]">
              {segment.label}
            </span>
            <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
              {fmt(segment.tokens)}
            </span>
          </div>
        ))}
        <div className="flex items-center gap-2">
          <span
            aria-hidden
            className="bg-foreground/[0.08] inset-ring-border size-2 shrink-0 rounded-full inset-ring forced-colors:border"
          />
          <span className="text-muted-foreground min-w-0 flex-1 truncate text-[13px]">
            {headroomLabel}
          </span>
          <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
            {fmt(Math.max(0, limit - used))}
          </span>
        </div>
        {stats.length > 0 && <div className="border-foreground/10 my-1 border-t" />}
        {stats.map(stat => (
          <div key={stat.label} className="flex items-center justify-between gap-4">
            <span className="text-muted-foreground min-w-0 truncate text-[12px]">{stat.label}</span>
            <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
              {stat.value}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
