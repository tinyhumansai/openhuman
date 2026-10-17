'use client';

/**
 * Vendored from the assistant-ui `elements-stopped-run` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-stopped-run.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - "Continue"/"Discard" are `continueLabel`/`discardLabel` props with
 *   English defaults, for `useT()`.
 * - No `words` paragraph: the stopped reply's text renders through the
 *   message's normal markdown part, so this element is only the reason chip
 *   and the Continue/Discard actions (#7729).
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { ArrowRightIcon, SquareIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, mono } from './surfaces';

export function StoppedRun({
  reason,
  onContinue,
  onDiscard,
  continueLabel = 'Continue',
  discardLabel = 'Discard',
  className,
  ...props
}: Omit<ComponentProps<'div'>, 'children' | 'reason' | 'onContinue' | 'onDiscard'> & {
  reason: string;
  onContinue?: () => void;
  onDiscard?: () => void;
  continueLabel?: string;
  discardLabel?: string;
}) {
  return (
    <div data-slot="stopped-run" className={cn('flex w-full flex-col gap-3', className)} {...props}>
      <div className="flex items-center gap-2">
        <span
          className={cn(
            field,
            mono,
            'text-muted-foreground inline-flex items-center gap-1.5 rounded-full px-2.5 py-1'
          )}>
          <SquareIcon className="size-2.5 fill-current" />
          {reason}
        </span>

        <button
          type="button"
          onClick={onContinue}
          className="text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/95 ms-auto flex h-7 items-center gap-1 rounded-full px-2.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]">
          {continueLabel}
          <ArrowRightIcon className="size-3" />
        </button>
        <button
          type="button"
          onClick={onDiscard}
          className="text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 flex h-7 items-center rounded-full px-2.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]">
          {discardLabel}
        </button>
      </div>
    </div>
  );
}
