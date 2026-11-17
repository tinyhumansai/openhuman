/**
 * The composer's message queue: the running prompt and the messages queued
 * behind it, rendered with assistant-ui's `message-queue` element.
 *
 * `ComposerPrimitive.Queue` supplies each item's scope; `QueueItemPrimitive`
 * renders its text and removes it through the core-backed queue adapter.
 */
import { ComposerPrimitive, QueueItemPrimitive, useAuiState } from '@assistant-ui/react';
import { XIcon } from 'lucide-react';

import {
  MessageQueue,
  MessageQueueItem,
} from '../../../components/assistant-ui/elements/message-queue';
import { Button } from '../../../components/assistant-ui/ui/button';
import { useT } from '../../../lib/i18n/I18nContext';

type ThreadMessages = ReadonlyArray<{
  role: string;
  content: ReadonlyArray<{ type: string; text?: string }>;
}>;

/** Text of the newest user message: the prompt the running turn answers. */
function runningPrompt(messages: ThreadMessages): string {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message.role !== 'user') continue;
    return message.content
      .map(part => (part.type === 'text' ? (part.text ?? '') : ''))
      .join('')
      .trim();
  }
  return '';
}

export function ComposerMessageQueue() {
  const { t } = useT();
  const queue = useAuiState(s => s.composer.queue);
  const running = useAuiState(s => runningPrompt(s.thread.messages as ThreadMessages));

  if (queue.length === 0) return null;

  return (
    <MessageQueue
      data-testid="queued-followups"
      className="mb-2 max-w-none"
      running={running}
      queuedCount={queue.length}
      runningLabel={t('chat.messageQueue.running')}
      queuedLabel={count => t('chat.messageQueue.queuedCount').replace('{count}', String(count))}
      pendingHint={t('chat.messageQueue.pendingHint')}>
      <ComposerPrimitive.Queue>{() => <RuntimeQueueItem />}</ComposerPrimitive.Queue>
    </MessageQueue>
  );
}

/** Queue item identity, text, and removal stay inside assistant-ui's item scope. */
function RuntimeQueueItem() {
  const { t } = useT();
  const text = useAuiState(s =>
    s.queueItem.parts
      .filter(part => part.type === 'text')
      .map(part => part.text)
      .join('\n\n')
  );
  const position = useAuiState(
    s => s.composer.queue.findIndex(item => item.id === s.queueItem.id) + 1
  );
  return (
    <MessageQueueItem
      position={position}
      text={<QueueItemPrimitive.Text />}
      action={
        <QueueItemPrimitive.Remove asChild>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={t('chat.messageQueue.remove').replace('{text}', text)}
            className="rounded-full">
            <XIcon aria-hidden className="size-3.5" />
          </Button>
        </QueueItemPrimitive.Remove>
      }
    />
  );
}
