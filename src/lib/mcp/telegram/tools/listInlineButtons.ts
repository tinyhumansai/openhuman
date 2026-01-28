import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById, getMessages } from '../telegramApi';

export const tool: MCPTool = {
  name: "list_inline_buttons",
  description: "List inline buttons on a message",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      message_id: { type: "number", description: "Message ID" },
    },
    required: ["chat_id", "message_id"],
  },
};

export async function listInlineButtons(
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

    const msg = messages.find((m) => String(m.id) === String(messageId)) as any;
    if (!msg) return { content: [{ type: 'text', text: 'Message ' + messageId + ' not found in cache.' }], isError: true };

    if (!msg.replyMarkup || !msg.replyMarkup.rows) {
      return { content: [{ type: 'text', text: 'No inline buttons on message ' + messageId + '.' }] };
    }

    const lines: string[] = [];
    msg.replyMarkup.rows.forEach((row: any, ri: number) => {
      if (row.buttons) {
        row.buttons.forEach((btn: any, bi: number) => {
          lines.push('Row ' + ri + ', Button ' + bi + ': "' + (btn.text ?? '?') + '"');
        });
      }
    });

    if (lines.length === 0) {
      return { content: [{ type: 'text', text: 'No inline buttons on message ' + messageId + '.' }] };
    }

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'list_inline_buttons',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
