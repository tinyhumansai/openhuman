import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById, getMessages, formatMessage } from '../telegramApi';

export const tool: MCPTool = {
  name: 'get_last_interaction',
  description: 'Get the last message exchanged with a user or chat',
  inputSchema: {
    type: 'object',
    properties: {
      chat_id: { type: 'string', description: 'Chat ID or username' },
    },
    required: ['chat_id'],
  },
};

export async function getLastInteraction(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');

    const chat = getChatById(chatId);
    if (!chat) {
      return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };
    }

    const messages = await getMessages(chatId, 1, 0);
    if (!messages || messages.length === 0) {
      return { content: [{ type: 'text', text: 'No messages found in this chat.' }] };
    }

    const msg = messages[0];
    const f = formatMessage(msg);
    const from = msg.fromName ?? msg.fromId ?? 'Unknown';

    return {
      content: [{
        type: 'text',
        text: 'Last message in ' + (chat.title ?? chatId) + ':\nFrom: ' + from + ' | Date: ' + f.date + ' | ' + (f.text || '[Media/No text]'),
      }],
    };
  } catch (error) {
    return logAndFormatError(
      'get_last_interaction',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
