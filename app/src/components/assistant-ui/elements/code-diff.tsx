'use client';

/**
 * assistant-ui's code-diff element: a file's added and removed lines.
 *
 * Vendored from assistant-ui `packages/ui/src/components/react/assistant-ui/elements/code-diff.tsx`
 * (commit 1abca347). Only the `cn` import path differs from upstream.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import type { ComponentProps } from 'react';

import { codeScroll, codeSurface, mono, paper } from './surfaces';

export type DiffKind = 'context' | 'added' | 'removed';

export interface DiffLine {
  kind: DiffKind;
  text: string;
}

const GUTTER: Record<DiffKind, string> = { context: '', added: '+', removed: '−' };

export function CodeDiff({
  filename,
  additions,
  deletions,
  lines,
  cycle,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'filename' | 'additions' | 'deletions' | 'lines' | 'cycle'
> & {
  filename: string;
  additions: number;
  deletions: number;
  lines: readonly DiffLine[];
  cycle: number;
}) {
  return (
    <div
      data-slot="code-diff"
      className={cn(paper, 'w-full overflow-hidden rounded-2xl font-mono text-xs', className)}
      {...props}>
      <div className="flex items-center justify-between px-4 pt-3 pb-2">
        <span className="text-foreground/90">{filename}</span>
        <span className={cn(mono, 'tabular-nums')}>
          <span className="text-emerald-600 dark:text-emerald-400">+{additions}</span>{' '}
          <span className="text-red-600 dark:text-red-400">−{deletions}</span>
        </span>
      </div>
      <div className={codeScroll}>
        <div className={codeSurface}>
          {lines.map((line, i) => (
            <div
              key={`${cycle}-${i}-${line.text}`}
              className={cn(
                'fade-in animate-in fill-mode-both flex px-4 py-0.5 leading-relaxed whitespace-pre duration-300',
                line.kind === 'context' && 'text-muted-foreground',
                line.kind === 'added' &&
                  'bg-emerald-500/10 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300',
                line.kind === 'removed' && 'bg-red-500/10 text-red-700 dark:text-red-300'
              )}
              style={{ animationDelay: `${i * 60}ms` }}>
              <span className="w-4 shrink-0 select-none">{GUTTER[line.kind]}</span>
              <span>{line.text}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
