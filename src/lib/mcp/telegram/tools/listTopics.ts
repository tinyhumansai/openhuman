import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'list_topics',
  description: 'List topics in a forum chat',
  inputSchema: { type: 'object', properties: { chat_id: { type: 'string', description: 'Chat ID or username' } }, required: ['chat_id'] },
};

export async function listTopics(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');

    const chat = getChatById(chatId);
    if (!chat) return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };

    if (chat.type !== 'channel' && chat.type !== 'supergroup') {
      return { content: [{ type: 'text', text: 'Forum topics are only available for channels/supergroups.' }], isError: true };
    }

    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      const inputChannel = await client.getInputEntity(entity);
      return client.invoke(
        new Api.channels.GetForumTopics({
          channel: inputChannel as Api.TypeInputChannel,
          offsetDate: 0,
          offsetId: 0,
          offsetTopic: 0,
          limit: 100,
        }),
      );
    });

    const topics = (result as any)?.topics;
    if (!topics || !Array.isArray(topics) || topics.length === 0) {
      return { content: [{ type: 'text', text: 'No forum topics found.' }] };
    }

    const lines = topics.map((t: any) => {
      return 'ID: ' + t.id + ' | ' + (t.title ?? 'Untitled');
    });

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'list_topics',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.GROUP,
    );
  }
}
