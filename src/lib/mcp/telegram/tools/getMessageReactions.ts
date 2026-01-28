import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'get_message_reactions',
  description: 'Get reactions on a message',
  inputSchema: {
    type: 'object',
    properties: {
      chat_id: { type: 'string', description: 'Chat ID or username' },
      message_id: { type: 'number', description: 'Message ID' },
    },
    required: ['chat_id', 'message_id'],
  },
};

export async function getMessageReactions(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');
    const messageId = typeof args.message_id === 'number' && Number.isInteger(args.message_id) ? args.message_id : undefined;

    if (messageId === undefined) {
      return { content: [{ type: 'text', text: 'message_id must be a positive integer' }], isError: true };
    }

    const chat = getChatById(chatId);
    if (!chat) return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };

    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      const inputPeer = await client.getInputEntity(entity);
      return client.invoke(
        new Api.messages.GetMessagesReactions({
          peer: inputPeer,
          id: [messageId],
        }),
      );
    });

    const updates = result as any;
    if (!updates || !updates.updates || updates.updates.length === 0) {
      return { content: [{ type: 'text', text: 'No reactions found on message ' + messageId + '.' }] };
    }

    const lines: string[] = [];
    for (const update of updates.updates) {
      if (update.reactions && update.reactions.results) {
        for (const r of update.reactions.results) {
          const emoji = r.reaction?.emoticon ?? r.reaction?.className ?? '?';
          const count = r.count ?? 0;
          lines.push(emoji + ': ' + count);
        }
      }
    }

    if (lines.length === 0) {
      return { content: [{ type: 'text', text: 'No reactions found on message ' + messageId + '.' }] };
    }

    return { content: [{ type: 'text', text: 'Reactions on message ' + messageId + ':\n' + lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_message_reactions',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
