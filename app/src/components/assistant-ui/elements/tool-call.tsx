'use client';

/**
 * assistant-ui's tool-call element: a disclosure row with a label that
 * shimmers while the call runs and swaps to its settled form, the call's
 * primary argument as a chip, and a Request / Result panel.
 *
 * Vendored from assistant-ui `packages/ui/src/components/react/assistant-ui/elements/tool-call.tsx`
 * (commit 1abca347). Changes from upstream:
 * - Radix collapsible (the app's), so the open-state selectors differ.
 * - Open state may be uncontrolled (`defaultOpen`).
 * - `icon`, `outcome` and `meta` slots: an OpenHuman call can fail, be
 *   cancelled or wait on the user, and carries a duration; upstream only
 *   knows running / done.
 * - `request` / `result` take nodes, and `children` replaces the panel body,
 *   so a call can expand into a richer element (terminal, diff, preview).
 * - `aside` renders between the row and the panel, always visible: a
 *   decision the turn is blocked on must not sit behind a disclosure.
 * - The panel headings are props, for translation.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/assistant-ui/ui/collapsible';
import { CheckIcon, ChevronRightIcon, CircleXIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import { collapsePanel, field, mono, openRotate, ShimmerLabel, SwapLabel } from './surfaces';

export type ToolCallOutcome = 'success' | 'error' | 'cancelled' | 'awaiting';

export interface ToolCallProps {
  label: string;
  activeLabel: string;
  query?: string;
  request?: ReactNode;
  result?: ReactNode;
  running: boolean;
  outcome?: ToolCallOutcome;
  icon?: ReactNode;
  meta?: ReactNode;
  aside?: ReactNode;
  children?: ReactNode;
  requestLabel?: string;
  resultLabel?: string;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  className?: string;
  'data-testid'?: string;
  'data-tool-name'?: string;
}

export function ToolCall({
  label,
  activeLabel,
  query,
  request,
  result,
  running,
  outcome = 'success',
  icon,
  meta,
  aside,
  children,
  requestLabel = 'Request',
  resultLabel = 'Result',
  open,
  defaultOpen,
  onOpenChange,
  className,
  ...props
}: ToolCallProps) {
  const hasPanel = children != null || request != null || result != null;
  return (
    <Collapsible
      data-slot="tool-call"
      data-outcome={running ? 'running' : outcome}
      open={open}
      defaultOpen={defaultOpen}
      onOpenChange={onOpenChange}
      className={cn('w-full ', className)}
      {...props}>
      <CollapsibleTrigger
        disabled={!hasPanel}
        className="group/trigger text-muted-foreground hover:text-foreground/90 flex w-full items-center gap-2 rounded-md py-1 text-[13.5px] transition-colors outline-none">
        <ChevronRightIcon
          className={cn('size-3.5 shrink-0 opacity-60', openRotate, !hasPanel && 'invisible')}
        />
        {icon}
        <SwapLabel active={running ? 0 : 1} className="shrink-0 text-start whitespace-nowrap">
          <ShimmerLabel
            active={running && outcome !== 'awaiting'}
            className="relative inline-block leading-none">
            {activeLabel}
          </ShimmerLabel>
          <>{label}</>
        </SwapLabel>
        {query ? (
          <span
            data-slot="tool-call-query"
            className={cn(
              mono,
              'bg-foreground/[0.06] text-muted-foreground min-w-0 truncate rounded-md px-1.5 py-0.5'
            )}>
            {query}
          </span>
        ) : null}
        <span className="ms-auto flex w-4 shrink-0 items-center justify-end">
          {meta}
          {!running && outcome === 'success' ? (
            <CheckIcon className="fade-in zoom-in-90 animate-in size-3.5 text-emerald-500 duration-200" />
          ) : null}
          {!running && (outcome === 'error' || outcome === 'cancelled') ? (
            <CircleXIcon className="fade-in zoom-in-90 animate-in size-3.5 text-red-500 duration-200" />
          ) : null}
        </span>
      </CollapsibleTrigger>
      {aside}
      {hasPanel ? (
        <CollapsibleContent className={cn(collapsePanel, 'outline-none')}>
          {children ?? (
            <div className={cn(field, 'mt-2 overflow-hidden rounded-2xl text-xs')}>
              {request != null ? (
                <div className="px-3.5 pt-2.5 pb-2">
                  <p className={cn(mono, 'text-muted-foreground mb-1')}>{requestLabel}</p>
                  <div className="text-muted-foreground max-h-48 overflow-auto font-mono">
                    {request}
                  </div>
                </div>
              ) : null}
              {request != null && result != null ? (
                <div className="bg-foreground/[0.06] mx-3.5 h-px" />
              ) : null}
              {result != null ? (
                <div className="px-3.5 pt-2 pb-2.5">
                  <p className={cn(mono, 'text-muted-foreground mb-1')}>{resultLabel}</p>
                  <div className="text-foreground/90 max-h-64 overflow-auto">{result}</div>
                </div>
              ) : null}
            </div>
          )}
        </CollapsibleContent>
      ) : null}
    </Collapsible>
  );
}
