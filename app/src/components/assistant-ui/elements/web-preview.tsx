'use client';

/**
 * assistant-ui's web-preview element: a URL bar with reload and open-in-new
 * around preview content.
 *
 * Vendored from assistant-ui `packages/ui/src/components/react/assistant-ui/elements/web-preview.tsx`
 * (commit 1abca347). Changes from upstream: the reload button renders only
 * when `onReload` is given (a fetched page has nothing to reload), and the
 * button and loading labels are props, for translation.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { ExternalLinkIcon, RotateCwIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, ghostButton, mono, paper, ShimmerLabel } from './surfaces';

/**
 * Chrome around a preview: a URL bar, reload, and open-in-new. It renders
 * `children` as given and enforces no isolation of its own, so the caller is
 * responsible for passing an already-sandboxed frame.
 */
export function WebPreview({
  origin,
  loading,
  children,
  onReload,
  onOpenExternal,
  reloadLabel = 'Reload the preview',
  openExternalLabel = 'Open the preview in a new tab',
  loadingLabel = 'Loading preview',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'origin' | 'loading' | 'children' | 'onReload' | 'onOpenExternal'
> & {
  origin: string;
  loading: boolean;
  children: React.ReactNode;
  onReload?: () => void;
  onOpenExternal?: () => void;
  reloadLabel?: string;
  openExternalLabel?: string;
  loadingLabel?: string;
}) {
  return (
    <div
      data-slot="web-preview"
      className={cn(paper, 'flex w-full flex-col overflow-hidden rounded-2xl', className)}
      {...props}>
      <div className="flex items-center gap-1.5 px-2.5 py-2">
        {onReload ? (
          <button
            type="button"
            aria-label={reloadLabel}
            onClick={onReload}
            className={cn(ghostButton, 'size-7 shrink-0')}>
            <RotateCwIcon
              className={cn('size-3.5', loading && 'animate-spin motion-reduce:animate-none')}
            />
          </button>
        ) : null}

        <span
          className={cn(
            field,
            'flex min-w-0 flex-1 items-center gap-1.5 rounded-full px-2.5 py-1'
          )}>
          <span className={cn(mono, 'text-muted-foreground min-w-0 truncate')}>{origin}</span>
        </span>

        <button
          type="button"
          aria-label={openExternalLabel}
          onClick={onOpenExternal}
          className={cn(ghostButton, 'size-7 shrink-0')}>
          <ExternalLinkIcon className="size-3.5" />
        </button>
      </div>

      <div className="border-foreground/[0.07] relative min-h-[9rem] border-t">
        <div
          aria-hidden={loading}
          className={cn(
            'transition-opacity duration-300 motion-reduce:transition-none',
            loading && 'invisible opacity-0'
          )}>
          {children}
        </div>
        {loading && (
          <div className="absolute inset-0 flex items-center justify-center">
            <ShimmerLabel className="text-muted-foreground relative inline-block text-xs leading-none">
              {loadingLabel}
            </ShimmerLabel>
          </div>
        )}
      </div>
    </div>
  );
}
