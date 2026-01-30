/**
 * List Chats tool - List available chats with metadata
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { optNumber, optString } from "../args";
import { formatEntity, getChatsWithApiFallback } from "../telegramApi";
import { toHumanReadableAction } from "../toolActionParser";

export const tool: MCPTool = {
  name: "list_chats",
  description: "List available chats with metadata",
  inputSchema: {
    type: "object",
    properties: {
      chat_type: {
        type: "string",
        enum: ["user", "group", "channel"],
        description:
          "Filter by chat type ('user', 'group', 'channel', or omit for all)",
      },
      limit: {
        type: "number",
        description: "Maximum number of chats to retrieve",
        default: 20,
      },
    },
  },
  toHumanReadableAction: (args) => toHumanReadableAction("list_chats", args),
};

export async function listChats(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const limit = optNumber(args, "limit", 20);
    const chatType = optString(args, "chat_type")?.toLowerCase();

    const chats = await getChatsWithApiFallback(limit);
    const contentItems: Array<{ type: "text"; text: string }> = [];

    for (const chat of chats) {
      const entity = formatEntity(chat);
      if (chatType && entity.type !== chatType) continue;

      let chatInfo = `Chat ID: ${entity.id}, Title: ${entity.name}, Type: ${entity.type}`;
      if (entity.username) chatInfo += `, Username: @${entity.username}`;
      chatInfo +=
        "unreadCount" in chat && chat.unreadCount
          ? `, Unread: ${chat.unreadCount}`
          : ", No unread messages";

      contentItems.push({ type: "text", text: chatInfo });
    }

    if (contentItems.length === 0) {
      return {
        content: [
          { type: "text", text: "No chats found matching the criteria." },
        ],
      };
    }

    return { content: contentItems };
  } catch (error) {
    return logAndFormatError(
      "list_chats",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CHAT,
    );
  }
}
