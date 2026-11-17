'use client';

/**
 * assistant-ui's tool-timeline element: a run of tool calls under one
 * disclosure whose label shimmers ("Working") while the run streams and
 * swaps to a summary once it settles.
 *
 * Vendored from assistant-ui `packages/ui/src/components/react/assistant-ui/elements/tool-timeline.tsx`
 * (commit 1abca347). Changes from upstream:
 * - Radix collapsible (the app's), so the open-state selectors differ.
 * - Open state may be uncontrolled (`defaultOpen`).
 * - `children` may replace `steps`: a live chat step is a full tool-call
 *   element (it expands, carries approvals), not only a verb and a chip.
 * - Step keys are positional; two steps may share a chip.
 * - `forceMount` on the content: Radix's default unmounts children entirely
 *   while closed, which both skips the CSS collapse animation
 *   (`animate-collapsible-up`/`-down` need the node present to measure) and,
 *   for a caller passing `children` that carry their own uncontrolled
 *   disclosure state (e.g. `ToolTimelineAdapter`'s per-row expand/collapse),
 *   would reset that state every time the outer group re-collapses. Staying
 *   mounted and letting `hidden`/the animation classes handle visibility
 *   matches the "hidden, never wiped" contract every other collapsible
 *   surface in this app already follows.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/assistant-ui/ui/collapsible';
import { ChevronRightIcon, type LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { take } from '../utils/range';
import { collapsePanel, openRotate, ShimmerLabel, SwapLabel } from './surfaces';

export interface TimelineStep {
  verb: string;
  chip: string;
  icon: LucideIcon;
}

export interface TimelineStat {
  file: string;
  added?: number;
  removed?: number;
}

export interface ToolTimelineProps {
  steps?: readonly TimelineStep[];
  visibleSteps?: number;
  children?: ReactNode;
  streaming: boolean;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  restingLabel: string;
  activeLabel: string;
  stats?: TimelineStat[];
  className?: string;
  'data-testid'?: string;
}

export function ToolTimeline({
  steps = [],
  visibleSteps = steps.length,
  children,
  streaming,
  open,
  defaultOpen,
  onOpenChange,
  restingLabel,
  activeLabel,
  stats = [],
  className,
  ...props
}: ToolTimelineProps) {
  return (
    <Collapsible
      data-slot="tool-timeline"
      open={open}
      defaultOpen={defaultOpen}
      onOpenChange={onOpenChange}
      className={cn('w-full ', className)}
      {...props}>
      <CollapsibleTrigger className="group/trigger text-muted-foreground hover:text-foreground/90 flex items-center gap-1.5 rounded-md py-1 text-[13.5px] transition-colors outline-none">
        <ChevronRightIcon className={cn('size-3.5 shrink-0 opacity-60', openRotate)} />
        <SwapLabel active={streaming ? 0 : 1} className="text-start tabular-nums">
          <ShimmerLabel active={streaming} className="relative inline-block leading-none">
            {activeLabel}
          </ShimmerLabel>
          <>{restingLabel}</>
        </SwapLabel>
      </CollapsibleTrigger>
      <CollapsibleContent forceMount className={cn(collapsePanel, 'outline-none')}>
        <div className="flex flex-col gap-2.5 ps-4 pt-2.5">
          {children ??
            take(steps, visibleSteps).map((step, index, shown) => {
              const Icon = step.icon;
              const active = streaming && index === shown.length - 1;

              return (
                <div
                  key={`${index}-${step.chip}`}
                  className="fade-in slide-in-from-bottom-1 animate-in fill-mode-both text-muted-foreground flex items-center gap-2 text-[13.5px] duration-300">
                  <Icon className="text-muted-foreground size-3.5 shrink-0" />
                  <ShimmerLabel active={active} className="relative inline-block leading-none">
                    {step.verb}
                  </ShimmerLabel>
                  <span className="bg-foreground/[0.06] text-muted-foreground rounded-md px-1.5 py-0.5 font-mono text-[11px]">
                    {step.chip}
                  </span>
                </div>
              );
            })}
          {stats.length > 0 && (
            <div className="flex flex-wrap gap-1.5 pt-1">
              {stats.map(stat => (
                <span
                  key={stat.file}
                  className="bg-foreground/[0.06] text-muted-foreground inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-mono text-[11px]">
                  <span>{stat.file}</span>
                  {stat.added !== undefined && (
                    <span className="text-emerald-600 dark:text-emerald-400">+{stat.added}</span>
                  )}
                  {stat.removed !== undefined && (
                    <span className="text-red-600 dark:text-red-400">−{stat.removed}</span>
                  )}
                </span>
              ))}
            </div>
          )}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
