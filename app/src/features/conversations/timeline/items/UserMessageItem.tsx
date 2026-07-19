import { BubbleMarkdown } from '../../components/AgentMessageBubble';

/**
 * Renders one user message bubble (text only). Attachment/reaction chrome is
 * layered on by the panel at swap time; this keeps the pure renderer focused on
 * the message text.
 */
export function UserMessageItem({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[80%] rounded-2xl bg-primary-500 px-4 py-2 text-sm text-white">
        <BubbleMarkdown content={content} />
      </div>
    </div>
  );
}
