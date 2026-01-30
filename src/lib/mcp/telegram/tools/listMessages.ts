/**
 * List Messages tool - Retrieve messages with optional filters
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { optNumber, optString } from "../args";
import {
  formatMessage,
  getChatById,
  getMessagesWithApiFallback,
} from "../telegramApi";
import { validateId } from "../../validation";

export const tool: MCPTool = {
  name: "list_messages",
  description: "Retrieve messages with optional filters",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: {
        type: "string",
        description: "The ID or username of the chat to get messages from",
      },
      limit: {
        type: "number",
        description: "Maximum number of messages to retrieve",
        default: 20,
      },
      search_query: {
        type: "string",
        description: "Filter messages containing this text",
      },
      from_date: {
        type: "string",
        description: "Filter messages from this date (YYYY-MM-DD)",
      },
      to_date: {
        type: "string",
        description: "Filter messages until this date (YYYY-MM-DD)",
      },
    },
    required: ["chat_id"],
  },
};

export async function listMessages(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, "chat_id");
    const limit = optNumber(args, "limit", 20);
    const searchQuery = optString(args, "search_query");
    const fromDate = optString(args, "from_date");
    const toDate = optString(args, "to_date");

    const chat = getChatById(chatId);
    if (!chat) {
      return {
        content: [{ type: "text", text: `Chat not found: ${chatId}` }],
        isError: true,
      };
    }

    let messages = await getMessagesWithApiFallback(chatId, limit * 2, 0);
    if (!messages || messages.length === 0) {
      return {
        content: [
          { type: "text", text: "No messages found matching the criteria." },
        ],
      };
    }

    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      messages = messages.filter((m) =>
        (m.message ?? "").toLowerCase().includes(q),
      );
    }

    if (fromDate || toDate) {
      messages = messages.filter((m) => {
        const d = new Date(m.date * 1000);
        if (fromDate && d < new Date(fromDate)) return false;
        if (toDate) {
          const to = new Date(toDate);
          to.setHours(23, 59, 59, 999);
          if (d > to) return false;
        }
        return true;
      });
    }

    const sliced = messages.slice(0, limit);
    const contentItems = sliced.map((msg) => {
      const formatted = formatMessage(msg);
      const from = msg.fromName ?? msg.fromId ?? "Unknown";
      const replyStr = msg.replyToMessageId
        ? ` | reply to ${msg.replyToMessageId}`
        : "";
      const text = `ID: ${formatted.id} | ${from} | Date: ${formatted.date}${replyStr} | Message: ${formatted.text || "[Media/No text]"}`;
      return { type: "text" as const, text };
    });

    return { content: contentItems };
  } catch (error) {
    return logAndFormatError(
      "list_messages",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
