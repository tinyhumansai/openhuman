import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "save_draft",
  description: "Save a draft in a chat",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      text: { type: "string", description: "Draft text" },
    },
    required: ["chat_id", "text"],
  },
};

export async function saveDraft(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');
    const text = typeof args.text === 'string' ? args.text : '';
    if (!text) return { content: [{ type: 'text', text: 'text is required' }], isError: true };

    const chat = getChatById(chatId);
    if (!chat) return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };

    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;

    await mtprotoService.withFloodWaitHandling(async () => {
      const inputPeer = await client.getInputEntity(entity);
      await client.invoke(
        new Api.messages.SaveDraft({
          peer: inputPeer,
          message: text,
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Draft saved in chat ' + chatId + '.' }] };
  } catch (error) {
    return logAndFormatError(
      'save_draft',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.DRAFT,
    );
  }
}
