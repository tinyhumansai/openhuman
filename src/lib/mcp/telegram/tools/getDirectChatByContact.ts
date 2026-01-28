import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { store } from '../../../../store';
import { selectOrderedChats } from '../../../../store/telegramSelectors';

export const tool: MCPTool = {
  name: 'get_direct_chat_by_contact',
  description: 'Get direct message chat with a contact',
  inputSchema: {
    type: 'object',
    properties: {
      user_id: { type: 'string', description: 'User ID' },
    },
    required: ['user_id'],
  },
};

export async function getDirectChatByContact(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const userId = validateId(args.user_id, 'user_id');
    const state = store.getState();
    const chats = selectOrderedChats(state);

    const dmChat = chats.find(
      (c) => c.type === 'private' && String(c.id) === String(userId),
    );

    if (!dmChat) {
      return { content: [{ type: 'text', text: 'No direct chat found with user ' + userId + '.' }] };
    }

    return {
      content: [{
        type: 'text',
        text: 'Chat ID: ' + dmChat.id + ' | Title: ' + (dmChat.title ?? 'DM') + ' | Username: ' + (dmChat.username ?? 'N/A'),
      }],
    };
  } catch (error) {
    return logAndFormatError(
      'get_direct_chat_by_contact',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
