'use client';

/**
 * Vendored from the assistant-ui `elements-guardrail-notice` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-guardrail-notice.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - "try instead" is an `alternativesLabel` prop with an English default,
 *   for `useT()`.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { ShieldIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { mono, paper } from './surfaces';

export function GuardrailNotice({
  title,
  explanation,
  policy,
  alternatives,
  onPick,
  alternativesLabel = 'try instead',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'title' | 'explanation' | 'policy' | 'alternatives' | 'onPick'
> & {
  title: string;
  explanation: string;
  policy: string;
  alternatives: readonly string[];
  onPick?: (alternative: string) => void;
  alternativesLabel?: string;
}) {
  return (
    <div
      data-slot="guardrail-notice"
      className={cn(paper, 'flex w-full flex-col gap-3 rounded-[20px] p-4', className)}
      {...props}>
      <div className="flex items-center gap-2.5">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-amber-500/12 text-amber-600 dark:text-amber-400">
          <ShieldIcon className="size-3.5" />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13.5px] font-medium">{title}</span>
        <span className={cn(mono, 'text-muted-foreground shrink-0')}>{policy}</span>
      </div>

      <p className="text-muted-foreground text-xs leading-relaxed">{explanation}</p>

      {alternatives.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <span className={cn(mono, 'text-muted-foreground')}>{alternativesLabel}</span>
          {alternatives.map(alternative =>
            onPick ? (
              <button
                key={alternative}
                type="button"
                onClick={() => onPick(alternative)}
                className="hover:bg-foreground/[0.04] text-muted-foreground hover:text-foreground/95 -mx-1.5 rounded-lg px-1.5 py-1 text-start text-[13px] transition-colors">
                {alternative}
              </button>
            ) : (
              <span
                key={alternative}
                className="text-muted-foreground -mx-1.5 rounded-lg px-1.5 py-1 text-start text-[13px]">
                {alternative}
              </span>
            )
          )}
        </div>
      )}
    </div>
  );
}
