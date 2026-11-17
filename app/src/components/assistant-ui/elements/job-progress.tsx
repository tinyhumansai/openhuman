'use client';

/**
 * Vendored from the assistant-ui `elements-job-progress` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-job-progress.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - `../utils/range` -> `@/components/assistant-ui/utils/range`.
 * - `aria-label="Cancel the job"` -> `cancelLabel` prop (English default
 *   matching upstream) so a host can supply `useT()`-sourced copy.
 * `title`/`stages[].name`/`eta` are caller-supplied props; nothing else here
 * is hard-coded user-facing copy.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { announced, clamp, pct, progressOf, take } from '@/components/assistant-ui/utils/range';
import { CheckIcon, Loader2Icon, XIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { ghostButton, mono, paper } from './surfaces';

export interface JobStage {
  name: string;
  weight: number;
}

export function JobProgress({
  title,
  stages,
  stageIndex,
  stageProgress,
  eta,
  onCancel,
  cancelLabel = 'Cancel the job',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'title' | 'stages' | 'stageIndex' | 'stageProgress' | 'eta' | 'onCancel'
> & {
  title: string;
  stages: readonly JobStage[];
  stageIndex: number;
  stageProgress: number;
  eta: string;
  onCancel?: () => void;
  cancelLabel?: string;
}) {
  const stage = progressOf(stageIndex, stages.length);
  const progress = clamp(stageProgress, 0, 1);
  const totalWeight = stages.reduce((sum, item) => sum + item.weight, 0) || 1;
  const completed = take(stages, stage).reduce((sum, item) => sum + item.weight, 0);
  const current = stages[stage];
  const overall = pct(completed + (current ? current.weight * progress : 0), totalWeight);
  const finished = stage >= stages.length;

  return (
    <div
      data-slot="job-progress"
      className={cn(paper, 'flex w-full flex-col gap-3 rounded-2xl p-4', className)}
      {...props}>
      <div className="flex items-center gap-2.5">
        {finished ? (
          <CheckIcon className="size-3.5 shrink-0 text-emerald-500" />
        ) : (
          <Loader2Icon className="text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
        )}
        <span className="min-w-0 flex-1 truncate text-[13.5px] font-medium">{title}</span>
        <span className={cn(mono, 'text-muted-foreground shrink-0 tabular-nums')}>
          {finished ? 'done' : eta}
        </span>
        {!finished && (
          <button
            type="button"
            aria-label={cancelLabel}
            onClick={onCancel}
            className={cn(ghostButton, 'size-6 shrink-0')}>
            <XIcon className="size-3.5" />
          </button>
        )}
      </div>

      <span
        role="progressbar"
        aria-label={`${title} progress`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={announced(overall)}
        className="bg-foreground/[0.06] inset-ring-border h-1 w-full overflow-hidden rounded-full inset-ring forced-colors:border">
        <span
          className={cn(
            'block h-full rounded-full transition-[width] duration-500 ease-out forced-color-adjust-none motion-reduce:transition-none',
            finished ? 'bg-emerald-500' : 'bg-blue-500 dark:bg-blue-400'
          )}
          style={{ width: `${overall}%` }}
        />
      </span>

      <div className="flex flex-wrap gap-x-3 gap-y-1">
        {stages.map((item, i) => (
          <span
            key={item.name}
            className={cn(
              mono,
              i < stage
                ? 'text-muted-foreground'
                : i === stage
                  ? 'text-foreground/90'
                  : 'text-muted-foreground'
            )}>
            {item.name}
          </span>
        ))}
      </div>
    </div>
  );
}
