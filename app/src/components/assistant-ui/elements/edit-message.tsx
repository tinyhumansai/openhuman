'use client';

/**
 * Vendored from the assistant-ui `elements-edit-message` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-edit-message.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - "Cancel"/"Send" are `cancelLabel`/`sendLabel` props; the "Edit your
 *   message" aria-label is `editAriaLabel`; all with English defaults, for
 *   `useT()`.
 * - The pluralized "sending discards N replies" copy is now a
 *   `discardedRepliesText(count)` template prop, defaulting to the upstream
 *   English copy.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import { AlertTriangleIcon } from 'lucide-react';
import type { ComponentProps } from 'react';

import { field, inkButton, mono, paper } from './surfaces';

const defaultDiscardedRepliesText = (count: number) =>
  `sending discards ${count} ${count === 1 ? 'reply' : 'replies'}`;

export function EditMessage({
  value,
  discardedReplies,
  editing,
  onValueChange,
  onSave,
  onCancel,
  onStartEdit,
  cancelLabel = 'Cancel',
  sendLabel = 'Send',
  editAriaLabel = 'Edit your message',
  discardedRepliesText = defaultDiscardedRepliesText,
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  | 'children'
  | 'value'
  | 'discardedReplies'
  | 'editing'
  | 'onValueChange'
  | 'onSave'
  | 'onCancel'
  | 'onStartEdit'
> & {
  value: string;
  discardedReplies: number;
  editing: boolean;
  onValueChange?: (value: string) => void;
  onSave?: () => void;
  onCancel?: () => void;
  onStartEdit?: () => void;
  cancelLabel?: string;
  sendLabel?: string;
  editAriaLabel?: string;
  discardedRepliesText?: (count: number) => string;
}) {
  if (!editing) {
    return (
      <div data-slot="edit-message" className={cn('flex w-full justify-end', className)} {...props}>
        <button
          type="button"
          onClick={onStartEdit}
          className={cn(
            field,
            'hover:bg-foreground/[0.07] max-w-[85%] rounded-2xl px-3.5 py-2.5 text-start text-[13.5px] transition-colors'
          )}>
          {value}
        </button>
      </div>
    );
  }

  return (
    <div
      data-slot="edit-message"
      className={cn(paper, 'flex w-full flex-col gap-3 rounded-[20px] p-3.5', className)}
      {...props}>
      <textarea
        value={value}
        onChange={event => onValueChange?.(event.target.value)}
        rows={2}
        aria-label={editAriaLabel}
        className={cn(
          field,
          'text-foreground/90 focus-visible:ring-foreground/20 resize-none rounded-xl px-3 py-2.5 text-[13.5px] leading-relaxed outline-none focus-visible:ring-1'
        )}
      />

      {discardedReplies > 0 && (
        <div className="flex items-center gap-2 text-amber-700 dark:text-amber-400">
          <AlertTriangleIcon className="size-3.5 shrink-0" />
          <span className={cn(mono, 'tabular-nums')}>{discardedRepliesText(discardedReplies)}</span>
        </div>
      )}

      <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 h-8 rounded-full px-3.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]">
          {cancelLabel}
        </button>
        <button
          type="button"
          onClick={onSave}
          className={cn(
            inkButton,
            'flex h-8 items-center rounded-full px-3.5 text-xs font-medium'
          )}>
          {sendLabel}
        </button>
      </div>
    </div>
  );
}
