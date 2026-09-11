/**
 * The conversation viewport is assistant-ui's core `ThreadPrimitive.Viewport`
 * when a runtime is available. The small DOM fallback keeps this shared shell
 * usable by isolated render tests and non-chat previews that intentionally do
 * not mount an assistant-ui runtime.
 */
import { type AssistantState, ThreadPrimitive, useAuiState } from '@assistant-ui/react';
import { forwardRef, type HTMLAttributes } from 'react';

import { cn } from '../../lib/cn';

export type ConversationProps = HTMLAttributes<HTMLDivElement>;

const selectHasThreadRuntime = (state: AssistantState) => state.optional.thread != null;

/**
 * The transcript's scroll viewport. Attach the ref returned by
 * `useStickToBottom` to keep it pinned as turns arrive.
 */
export const Conversation = forwardRef<HTMLDivElement, ConversationProps>(
  ({ className, ...props }, ref) => {
    const hasThreadRuntime = useAuiState(selectHasThreadRuntime);
    const viewportProps = {
      ref,
      'data-slot': 'conversation',
      role: 'log',
      className: cn('relative min-h-0 flex-1 overflow-y-auto', className),
      ...props,
    };

    // OpenHuman's richer transcript has tool timelines and streamed content
    // outside assistant-ui's message list. Its existing hook remains the
    // scroll authority; this viewport supplies assistant-ui's thread context
    // and future core affordances without competing auto-scroll behavior.
    return hasThreadRuntime ? (
      <ThreadPrimitive.Root className="contents">
        <ThreadPrimitive.Viewport autoScroll={false} {...viewportProps} />
      </ThreadPrimitive.Root>
    ) : (
      <div {...viewportProps} />
    );
  }
);
Conversation.displayName = 'Conversation';

export type ConversationContentProps = HTMLAttributes<HTMLDivElement>;

/** The column of turns inside the viewport. */
export const ConversationContent = forwardRef<HTMLDivElement, ConversationContentProps>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      data-slot="conversation-content"
      className={cn('flex w-full flex-col', className)}
      {...props}
    />
  )
);
ConversationContent.displayName = 'ConversationContent';
