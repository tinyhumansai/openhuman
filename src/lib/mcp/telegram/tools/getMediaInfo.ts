import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById, getMessages } from '../telegramApi';

export const tool: MCPTool = {
  name: "get_media_info",
  description: "Get media info from a message",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      message_id: { type: "number", description: "Message ID" },
    },
    required: ["chat_id", "message_id"],
  },
};

export async function getMediaInfo(
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

    const messages = await getMessages(chatId, 200, 0);
    if (!messages) return { content: [{ type: 'text', text: 'No messages found.' }] };

    const msg = messages.find((m) => String(m.id) === String(messageId));
    if (!msg) return { content: [{ type: 'text', text: 'Message ' + messageId + ' not found in cache.' }], isError: true };

    if (!msg.media) {
      return { content: [{ type: 'text', text: 'No media in message ' + messageId + '.' }] };
    }

    const info = {
      type: msg.media.type ?? 'unknown',
      ...msg.media,
    };

    return { content: [{ type: 'text', text: JSON.stringify(info, null, 2) }] };
  } catch (error) {
    return logAndFormatError(
      'get_media_info',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MEDIA,
    );
  }
}
