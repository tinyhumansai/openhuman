/**
 * Adapted from `message.tsx` in vercel/ai-elements (Apache-2.0).
 * Upstream: https://github.com/vercel/ai-elements — registry component `message`.
 *
 * Port notes (this is an adaptation, not a copy):
 * - `UIMessage["role"]` from the `ai` SDK → the local `MessageRole` union. The
 *   `ai` package is not a dependency here; the transcript only ever renders a
 *   user turn or an assistant turn, so a two-value union is the whole surface.
 *   The app's own wire vocabulary calls the assistant `'agent'`, so both
 *   spellings are accepted and normalised to `data-from="assistant"` — callers
 *   pass `msg.sender` straight through without a translation step.
 * - `@/lib/utils` `cn` → `../../lib/cn`; `@/registry/default/ui/button` →
 *   OpenHuman's `Button` from `../ui`.
 * - Upstream distinguishes the two sides with the marker classes `is-user` /
 *   `is-assistant` and then styles descendants through
 *   `group-[.is-user]:…` selectors. That reads as dead weight in a codebase
 *   whose tests are forbidden from asserting on class strings, so the marker is
 *   `data-from` instead and the layout difference (`justify-end` vs
 *   `justify-start`) is applied directly on the row.
 * - The group is named `group/msg`, not the bare `group` upstream uses. The
 *   transcript's hover affordances (the copy button, the reaction opener) are
 *   already written against `group-hover/msg:`, and a named group is what lets
 *   a message row nest inside other `group`s without capturing their hovers.
 * - `MessageContent` drops upstream's bubble chrome
 *   (`group-[.is-user]:rounded-lg …bg-secondary`): in this app the bubble is
 *   painted by `AgentMessageBubble` / the user bubble in `TranscriptRow`, which
 *   have their own token-based treatment, corner-tucking and attachment rows.
 *   What is kept is the part the transcript actually needs from it — a
 *   `relative`, min-width-0 column that the absolutely-positioned actions can
 *   anchor against.
 * - `MessageAction` renders an `aria-label` rather than upstream's
 *   `<span className="sr-only">`, because this repo requires an accessible name
 *   on every icon-only button and an `aria-label` is the one screen readers
 *   announce as the button's name rather than as its content. The upstream
 *   `tooltip` prop and its `TooltipProvider` wrapper are NOT ported: the
 *   transcript's actions are absolutely positioned against
 *   `MessageContent`, and an extra trigger wrapper between them and their
 *   positioning context would break that anchor. `title` covers the hover hint.
 * - `MessageBranch*` is not ported. This app renders concurrent (forked) turns
 *   as separate labelled bubbles rather than as one switchable slot, so the
 *   branch selector would be an unused port.
 * - `MessageResponse` is not ported: it is a thin wrapper over `streamdown`,
 *   which is not a dependency here. Markdown rendering goes through
 *   `features/conversations/components/AgentMessageBubble`'s `BubbleMarkdown`.
 * - `data-slot` attributes added throughout so tests assert structure rather
 *   than class strings.
 */
import { type ComponentProps, forwardRef, type HTMLAttributes } from 'react';

import { cn } from '../../lib/cn';
import { Button } from '../ui';

/**
 * Who produced the turn. `'agent'` is this app's own spelling of the assistant
 * side and is treated as an alias for `'assistant'`.
 */
export type MessageRole = 'user' | 'assistant' | 'agent';

export type MessageProps = HTMLAttributes<HTMLDivElement> & { from: MessageRole };

/**
 * One row of the transcript: a full-width flex line that pushes its content to
 * the trailing edge for a user turn and the leading edge for an assistant turn.
 */
export const Message = forwardRef<HTMLDivElement, MessageProps>(
  ({ className, from, ...props }, ref) => {
    const isUser = from === 'user';
    return (
      <div
        ref={ref}
        data-slot="message"
        data-from={isUser ? 'user' : 'assistant'}
        className={cn('group/msg flex w-full', isUser ? 'justify-end' : 'justify-start', className)}
        {...props}
      />
    );
  }
);
Message.displayName = 'Message';

export type MessageContentProps = HTMLAttributes<HTMLDivElement>;

/**
 * The turn's body column. `relative` is load-bearing: it is the positioning
 * context every `MessageAction` anchors to.
 */
export const MessageContent = forwardRef<HTMLDivElement, MessageContentProps>(
  ({ className, children, ...props }, ref) => (
    <div
      ref={ref}
      data-slot="message-content"
      className={cn('relative flex min-w-0 flex-col', className)}
      {...props}>
      {children}
    </div>
  )
);
MessageContent.displayName = 'MessageContent';

export type MessageActionsProps = HTMLAttributes<HTMLDivElement>;

/** A cluster of hover affordances for one turn. */
export const MessageActions = ({ className, children, ...props }: MessageActionsProps) => (
  <div data-slot="message-actions" className={cn('flex items-center gap-1', className)} {...props}>
    {children}
  </div>
);

export type MessageActionProps = Omit<ComponentProps<typeof Button>, 'aria-label'> & {
  /** Accessible name. Required — these buttons are always icon-only. */
  label: string;
};

/**
 * A single icon affordance on a message (copy, share, react …). Sized `xs` and
 * `tertiary` so it reads as chrome rather than as an action the turn is
 * offering.
 */
export const MessageAction = forwardRef<HTMLButtonElement, MessageActionProps>(
  ({ label, children, className, variant = 'tertiary', size = 'xs', ...props }, ref) => (
    <Button
      ref={ref}
      data-slot="message-action"
      type="button"
      variant={variant}
      size={size}
      iconOnly
      aria-label={label}
      title={label}
      className={className}
      {...props}>
      {children}
    </Button>
  )
);
MessageAction.displayName = 'MessageAction';
