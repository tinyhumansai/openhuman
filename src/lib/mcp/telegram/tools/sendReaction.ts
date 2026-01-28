import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'send_reaction',
  description: 'Send a reaction to a message',
  inputSchema: {
    type: 'object',
    properties: {
      chat_id: { type: 'string', description: 'Chat ID or username' },
      message_id: { type: 'number', description: 'Message ID' },
      reaction: { type: 'string', description: 'Reaction emoji' },
    },
    required: ['chat_id', 'message_id'],
  },
};

export async function sendReaction(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');
    const messageId = typeof args.message_id === 'number' && Number.isInteger(args.message_id) ? args.message_id : undefined;
    const emoji = typeof args.reaction === 'string' ? args.reaction : '\ud83d\udc4d';

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
          reaction: [new Api.ReactionEmoji({ emoticon: emoji })],
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Reaction ' + emoji + ' sent to message ' + messageId + '.' }] };
  } catch (error) {
    return logAndFormatError(
      'send_reaction',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
