/**
 * Get Chats tool - Get a paginated list of chats
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { optNumber } from "../args";
import { formatEntity, getChatsWithApiFallback } from "../telegramApi";

export const tool: MCPTool = {
  name: "get_chats",
  description: "Get a paginated list of chats",
  inputSchema: {
    type: "object",
    properties: {
      page: {
        type: "number",
        description: "Page number (1-indexed)",
        default: 1,
      },
      page_size: {
        type: "number",
        description: "Number of chats per page",
        default: 20,
      },
    },
  },
};

export async function getChats(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const page = optNumber(args, "page", 1);
    const pageSize = optNumber(args, "page_size", 20);
    const start = (page - 1) * pageSize;

    const chats = await getChatsWithApiFallback(pageSize + start);
    const paginatedChats = chats.slice(start, start + pageSize);

    if (paginatedChats.length === 0) {
      return { content: [{ type: "text", text: "Page out of range." }] };
    }

    const lines = paginatedChats.map((chat) => {
      const entity = formatEntity(chat);
      return `Chat ID: ${entity.id}, Title: ${entity.name}`;
    });

    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (error) {
    return logAndFormatError(
      "get_chats",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CHAT,
    );
  }
}
