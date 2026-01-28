import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'remove_reaction',
  description: 'Remove a reaction from a message',
  inputSchema: {
    type: 'object',
    properties: {
      chat_id: { type: 'string', description: 'Chat ID or username' },
      message_id: { type: 'number', description: 'Message ID' },
      reaction: { type: 'string', description: 'Reaction to remove' },
    },
    required: ['chat_id', 'message_id'],
  },
};

export async function removeReaction(
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

    await mtprotoService.withFloodWaitHandling(async () => {
      const inputPeer = await client.getInputEntity(entity);
      await client.invoke(
        new Api.messages.SendReaction({
          peer: inputPeer,
          msgId: messageId,
          reaction: [],
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Reaction removed from message ' + messageId + '.' }] };
  } catch (error) {
    return logAndFormatError(
      'remove_reaction',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
