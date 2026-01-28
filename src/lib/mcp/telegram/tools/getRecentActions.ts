import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';
import { optNumber } from '../args';

export const tool: MCPTool = {
  name: "get_recent_actions",
  description: "Get recent admin actions in a chat",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      limit: { type: "number", description: "Max actions", default: 20 },
    },
    required: ["chat_id"],
  },
};

export async function getRecentActions(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');
    const limit = optNumber(args, 'limit', 20);

    const chat = getChatById(chatId);
    if (!chat) return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };

    if (chat.type !== 'channel' && chat.type !== 'supergroup') {
      return { content: [{ type: 'text', text: 'Recent actions are only available for channels/supergroups.' }], isError: true };
    }

    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      const inputChannel = await client.getInputEntity(entity);
      return client.invoke(
        new Api.channels.GetAdminLog({
          channel: inputChannel as Api.TypeInputChannel,
          q: '',
          maxId: BigInt(0),
          minId: BigInt(0),
          limit,
        }),
      );
    });

    const events = (result as any)?.events;
    if (!events || !Array.isArray(events) || events.length === 0) {
      return { content: [{ type: 'text', text: 'No recent actions found.' }] };
    }

    const lines = events.map((e: any) => {
      const date = e.date ? new Date(e.date * 1000).toISOString() : 'unknown';
      const action = e.action?.className ?? 'unknown';
      return date + ' | ' + action;
    });

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_recent_actions',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.ADMIN,
    );
  }
}
