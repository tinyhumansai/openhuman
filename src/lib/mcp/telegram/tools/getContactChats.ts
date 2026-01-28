import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { store } from '../../../../store';
import { selectOrderedChats } from '../../../../store/telegramSelectors';

export const tool: MCPTool = {
  name: 'get_contact_chats',
  description: 'Get all chats that are direct messages with contacts',
  inputSchema: { type: 'object', properties: {} },
};

export async function getContactChats(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const state = store.getState();
    const chats = selectOrderedChats(state);
    const dmChats = chats.filter((c) => c.type === 'private');

    if (dmChats.length === 0) {
      return { content: [{ type: 'text', text: 'No contact chats found.' }] };
    }

    const lines = dmChats.map((c) => {
      const username = c.username ? '@' + c.username : '';
      return ('ID: ' + c.id + ' | ' + (c.title ?? 'DM') + ' ' + username).trim();
    });

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_contact_chats',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
