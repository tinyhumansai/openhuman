'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import {
  ComposerPrimitive,
  type QuoteMessagePartComponent,
  SelectionToolbarPrimitive,
} from '@assistant-ui/react';
import { QuoteIcon, XIcon } from 'lucide-react';
import { type ComponentProps, type FC, memo } from 'react';

function QuoteBlockRoot({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="quote-block"
      className={cn('mb-2 flex items-start gap-1.5', className)}
      {...props}
    />
  );
}

function QuoteBlockIcon({ className, ...props }: ComponentProps<typeof QuoteIcon>) {
  return (
    <QuoteIcon
      data-slot="quote-block-icon"
      className={cn('text-muted-foreground/60 mt-0.5 size-3 shrink-0', className)}
      {...props}
    />
  );
}

function QuoteBlockText({ className, ...props }: ComponentProps<'p'>) {
  return (
    <p
      data-slot="quote-block-text"
      className={cn('text-muted-foreground/80 line-clamp-2 min-w-0 text-sm italic', className)}
      {...props}
    />
  );
}

/**
 * Renders quoted text in user messages.
 *
 * Pass this to `MessagePrimitive.Parts` as the `Quote` renderer.
 *
 * @example
 * ```tsx
 * <MessagePrimitive.Quote>
 *   {(quote) => <QuoteBlock {...quote} />}
 * </MessagePrimitive.Quote>
 * ```
 */
const QuoteBlockImpl: QuoteMessagePartComponent = ({ text }) => {
  return (
    <QuoteBlockRoot>
      <QuoteBlockIcon />
      <QuoteBlockText>{text}</QuoteBlockText>
    </QuoteBlockRoot>
  );
};

const QuoteBlock = memo(QuoteBlockImpl) as unknown as QuoteMessagePartComponent & {
  Root: typeof QuoteBlockRoot;
  Icon: typeof QuoteBlockIcon;
  Text: typeof QuoteBlockText;
};

QuoteBlock.displayName = 'QuoteBlock';
QuoteBlock.Root = QuoteBlockRoot;
QuoteBlock.Icon = QuoteBlockIcon;
QuoteBlock.Text = QuoteBlockText;

function SelectionToolbarRoot({
  className,
  ...props
}: ComponentProps<typeof SelectionToolbarPrimitive.Root>) {
  return (
    <SelectionToolbarPrimitive.Root
      data-slot="selection-toolbar"
      className={cn('bg-popover flex items-center gap-1 rounded-lg border px-1 py-1', className)}
      {...props}
    />
  );
}

function SelectionToolbarQuote({
  className,
  children,
  ...props
}: ComponentProps<typeof SelectionToolbarPrimitive.Quote>) {
  return (
    <SelectionToolbarPrimitive.Quote
      data-slot="selection-toolbar-quote"
      className={cn(
        'text-popover-foreground hover:bg-accent flex items-center gap-1.5 rounded-md px-2.5 py-1 text-sm transition-colors',
        className
      )}
      {...props}>
      {children ?? (
        <>
          <QuoteIcon className="size-3.5" />
          Quote
        </>
      )}
    </SelectionToolbarPrimitive.Quote>
  );
}

/**
 * Floating toolbar that appears when text is selected in a message.
 *
 * Render anywhere inside `ThreadPrimitive.Root` (or any `AssistantRuntimeProvider` scope).
 *
 * @example
 * ```tsx
 * <ThreadPrimitive.Root>
 *   <ThreadPrimitive.Viewport>...</ThreadPrimitive.Viewport>
 *   <SelectionToolbar />
 * </ThreadPrimitive.Root>
 * ```
 */
const SelectionToolbarImpl: FC<ComponentProps<typeof SelectionToolbarRoot>> = ({
  className,
  ...props
}) => {
  return (
    <SelectionToolbarRoot className={className} {...props}>
      <SelectionToolbarQuote />
    </SelectionToolbarRoot>
  );
};

const SelectionToolbar = memo(SelectionToolbarImpl) as unknown as typeof SelectionToolbarImpl & {
  Root: typeof SelectionToolbarRoot;
  Quote: typeof SelectionToolbarQuote;
};

SelectionToolbar.displayName = 'SelectionToolbar';
SelectionToolbar.Root = SelectionToolbarRoot;
SelectionToolbar.Quote = SelectionToolbarQuote;

function ComposerQuotePreviewRoot({
  className,
  ...props
}: ComponentProps<typeof ComposerPrimitive.Quote>) {
  return (
    <ComposerPrimitive.Quote
      data-slot="composer-quote"
      className={cn('bg-muted/60 mx-3 mt-2 flex items-start gap-2 rounded-lg px-3 py-2', className)}
      {...props}
    />
  );
}

function ComposerQuotePreviewIcon({ className, ...props }: ComponentProps<typeof QuoteIcon>) {
  return (
    <QuoteIcon
      data-slot="composer-quote-icon"
      className={cn('text-muted-foreground/70 mt-0.5 size-3.5 shrink-0', className)}
      {...props}
    />
  );
}

function ComposerQuotePreviewText({
  className,
  ...props
}: ComponentProps<typeof ComposerPrimitive.QuoteText>) {
  return (
    <ComposerPrimitive.QuoteText
      data-slot="composer-quote-text"
      className={cn('text-muted-foreground line-clamp-2 min-w-0 flex-1 text-sm', className)}
      {...props}
    />
  );
}

function ComposerQuotePreviewDismiss({
  className,
  children,
  ...props
}: ComponentProps<typeof ComposerPrimitive.QuoteDismiss>) {
  const defaultClassName =
    'shrink-0 rounded-sm p-0.5 text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground';

  return (
    <ComposerPrimitive.QuoteDismiss
      data-slot="composer-quote-dismiss"
      className={children ? className : undefined}
      {...props}>
      {children ?? (
        <button
          type="button"
          aria-label="Dismiss quote"
          className={cn(defaultClassName, className)}>
          <XIcon className="size-3.5" />
        </button>
      )}
    </ComposerPrimitive.QuoteDismiss>
  );
}

/**
 * Quote preview inside the composer. Only renders when a quote is set.
 *
 * Place inside `ComposerPrimitive.Root`.
 *
 * @example
 * ```tsx
 * <ComposerPrimitive.Root>
 *   <ComposerQuotePreview />
 *   <ComposerPrimitive.Input />
 *   <ComposerPrimitive.Send />
 * </ComposerPrimitive.Root>
 * ```
 */
const ComposerQuotePreviewImpl: FC<ComponentProps<typeof ComposerQuotePreviewRoot>> = ({
  className,
  ...props
}) => {
  return (
    <ComposerQuotePreviewRoot className={className} {...props}>
      <ComposerQuotePreviewIcon />
      <ComposerQuotePreviewText />
      <ComposerQuotePreviewDismiss />
    </ComposerQuotePreviewRoot>
  );
};

const ComposerQuotePreview = memo(
  ComposerQuotePreviewImpl
) as unknown as typeof ComposerQuotePreviewImpl & {
  Root: typeof ComposerQuotePreviewRoot;
  Icon: typeof ComposerQuotePreviewIcon;
  Text: typeof ComposerQuotePreviewText;
  Dismiss: typeof ComposerQuotePreviewDismiss;
};

ComposerQuotePreview.displayName = 'ComposerQuotePreview';
ComposerQuotePreview.Root = ComposerQuotePreviewRoot;
ComposerQuotePreview.Icon = ComposerQuotePreviewIcon;
ComposerQuotePreview.Text = ComposerQuotePreviewText;
ComposerQuotePreview.Dismiss = ComposerQuotePreviewDismiss;

export {
  QuoteBlock,
  QuoteBlockRoot,
  QuoteBlockIcon,
  QuoteBlockText,
  SelectionToolbar,
  SelectionToolbarRoot,
  SelectionToolbarQuote,
  ComposerQuotePreview,
  ComposerQuotePreviewRoot,
  ComposerQuotePreviewIcon,
  ComposerQuotePreviewText,
  ComposerQuotePreviewDismiss,
};
