import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "press_inline_button",
  description: "Press an inline button on a message",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      message_id: { type: "number", description: "Message ID" },
      button_text: { type: "string", description: "Button text or data" },
    },
    required: ["chat_id", "message_id"],
  },
};

export async function pressInlineButton(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');
    const messageId = typeof args.message_id === 'number' && Number.isInteger(args.message_id) ? args.message_id : undefined;
    const data = typeof args.button_text === 'string' ? args.button_text : '';

    if (messageId === undefined) {
      return { content: [{ type: 'text', text: 'message_id must be a positive integer' }], isError: true };
    }
    if (!data) return { content: [{ type: 'text', text: 'button_text is required' }], isError: true };

    const chat = getChatById(chatId);
    if (!chat) return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };

    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      const inputPeer = await client.getInputEntity(entity);
      return client.invoke(
        new Api.messages.GetBotCallbackAnswer({
          peer: inputPeer,
          msgId: messageId,
          data: Buffer.from(data, 'base64'),
        }),
      );
    });

    const answer = (result as any)?.message ?? 'Button pressed (no response message).';
    return { content: [{ type: 'text', text: answer }] };
  } catch (error) {
    return logAndFormatError(
      'press_inline_button',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
