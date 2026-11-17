'use client';

/**
 * Vendored from the assistant-ui `task-card` registry item
 * (https://r.assistant-ui.com/styles/base-nova/task-card.json). Changes from
 * upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`), matching every
 *   other vendored element in this directory.
 * - the screen-reader state label goes through `useT()` instead of echoing the
 *   raw state id. Every other string (`label`, `meta`, `elapsed`, `result`) is a
 *   caller-supplied prop.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { useT } from '@/lib/i18n/I18nContext';
import {
  AlertTriangleIcon,
  Ban,
  CheckIcon,
  ChevronRightIcon,
  Loader2Icon,
  XIcon,
} from 'lucide-react';
import { Children, type ComponentProps, type ReactNode, useContext, useState } from 'react';
import { createPortal } from 'react-dom';

import { TaskCardDockTarget } from '../lib/task-card-dock';
import { mono, paper } from './surfaces';

export type TaskCardState = 'working' | 'waiting' | 'done' | 'failed' | 'cancelled' | 'incomplete';

const isRenderable = (node: ReactNode) =>
  node !== undefined && node !== null && node !== false && node !== true;

export function TaskStateIcon({ state, className }: { state: TaskCardState; className?: string }) {
  if (state === 'done') {
    return (
      <CheckIcon aria-hidden className={cn('size-3.5 shrink-0 text-emerald-500', className)} />
    );
  }
  if (state === 'failed') {
    return <XIcon aria-hidden className={cn('text-destructive size-3.5 shrink-0', className)} />;
  }
  if (state === 'incomplete') {
    return (
      <AlertTriangleIcon
        aria-hidden
        className={cn('size-3.5 shrink-0 text-amber-500', className)}
      />
    );
  }
  if (state === 'cancelled') {
    return <Ban aria-hidden className={cn('text-muted-foreground size-3.5 shrink-0', className)} />;
  }
  if (state === 'working') {
    return (
      <Loader2Icon
        aria-hidden
        className={cn(
          'text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none',
          className
        )}
      />
    );
  }
  return (
    <span
      aria-hidden
      className={cn('border-foreground/35 m-1 size-1.5 shrink-0 rounded-full border', className)}
    />
  );
}

export function TaskCard({
  label,
  meta,
  state,
  elapsed,
  actions,
  result,
  open,
  onOpenChange,
  children,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  'children' | 'label' | 'state' | 'result' | 'open' | 'onOpenChange'
> & {
  label: string;
  meta?: string | undefined;
  state: TaskCardState;
  elapsed?: string | undefined;
  actions?: ReactNode | undefined;
  result?: ReactNode | undefined;
  open?: boolean | undefined;
  onOpenChange?: ((open: boolean) => void) | undefined;
  children?: ReactNode | undefined;
}) {
  const { t } = useT();
  const dock = useContext(TaskCardDockTarget);
  const hasTranscript = Children.toArray(children).length > 0;
  const inert = open !== undefined && onOpenChange === undefined;
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const isOpen = open ?? uncontrolledOpen;
  const toggle = () => {
    const next = !isOpen;
    if (open === undefined) setUncontrolledOpen(next);
    onOpenChange?.(next);
  };

  const card = (
    <div
      data-slot="task-card"
      data-state={state}
      className={cn(
        paper,
        'flex w-full flex-col overflow-hidden rounded-2xl',
        (!dock || (state !== 'working' && state !== 'waiting')) && 'my-3',
        className
      )}
      {...props}>
      <button
        type="button"
        aria-expanded={hasTranscript ? isOpen : undefined}
        disabled={!hasTranscript || inert}
        onClick={toggle}
        className="hover:enabled:bg-foreground/[0.03] flex items-center gap-2.5 px-3.5 py-2.5 text-start transition-colors disabled:cursor-default">
        <TaskStateIcon state={state} />
        <span className="sr-only">{t(`conversations.taskCard.state.${state}`)}</span>
        <span className="min-w-0 flex-1 truncate text-sm">{label}</span>
        {meta !== undefined && (
          <span className={cn(mono, 'text-muted-foreground text-xs max-w-32 shrink-0 truncate')}>
            {meta}
          </span>
        )}
        {elapsed !== undefined && (
          <span className={cn(mono, 'text-muted-foreground text-xs shrink-0 tabular-nums')}>
            {elapsed}
          </span>
        )}
        {hasTranscript && (
          <ChevronRightIcon
            aria-hidden
            className={cn(
              'text-muted-foreground size-3 shrink-0 transition-transform duration-200 motion-reduce:transition-none',
              isOpen && 'rotate-90'
            )}
          />
        )}
      </button>
      {isRenderable(actions) && (
        <div data-slot="task-card-actions" className="border-border/60 border-t px-3.5 py-2.5">
          {actions}
        </div>
      )}
      {hasTranscript && isOpen && (
        <div
          data-slot="task-card-transcript"
          className="border-border/60 flex flex-col gap-2 border-t px-3.5 py-2.5">
          {children}
        </div>
      )}
      {isRenderable(result) && (
        <div
          data-slot="task-card-result"
          className="border-border/60 text-muted-foreground border-t px-3.5 py-2 text-xs leading-relaxed">
          {result}
        </div>
      )}
    </div>
  );
  return dock && (state === 'working' || state === 'waiting') ? createPortal(card, dock) : card;
}
