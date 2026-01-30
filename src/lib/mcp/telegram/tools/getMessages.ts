/**
 * Get Messages tool - Get paginated messages from a specific chat
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import {
  formatMessage,
  getChatById,
  getMessagesWithApiFallback,
} from "../telegramApi";
import { validateId } from "../../validation";
import type { TelegramMessage } from "../../../../store/telegram/types";
import { optNumber } from "../args";

export const tool: MCPTool = {
  name: "get_messages",
  description: "Get paginated messages from a specific chat",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: {
        type: "string",
        description: "The ID or username of the chat",
      },
      page: {
        type: "number",
        description: "Page number (1-indexed)",
        default: 1,
      },
      page_size: {
        type: "number",
        description: "Number of messages per page",
        default: 20,
      },
    },
    required: ["chat_id"],
  },
};

function senderDisplay(msg: TelegramMessage): string {
  return msg.fromName ?? msg.fromId ?? "Unknown";
}

export async function getMessages(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, "chat_id");
    const page = optNumber(args, "page", 1);
    const pageSize = optNumber(args, "page_size", 20);
    const offset = (page - 1) * pageSize;

    const chat = getChatById(chatId);
    if (!chat) {
      return {
        content: [{ type: "text", text: `Chat not found: ${chatId}` }],
        isError: true,
      };
    }

    const messages = await getMessagesWithApiFallback(chatId, pageSize, offset);
    if (!messages || messages.length === 0) {
      return {
        content: [{ type: "text", text: "No messages found for this page." }],
      };
    }

    const lines = messages.map((msg) => {
      const formatted = formatMessage(msg);
      const replyStr = msg.replyToMessageId
        ? ` | reply to ${msg.replyToMessageId}`
        : "";
      const msgText = formatted.text || "[Media/No text]";
      return `ID: ${formatted.id} | ${senderDisplay(msg)} | Date: ${formatted.date}${replyStr} | Message: ${msgText}`;
    });

    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (error) {
    return logAndFormatError(
      "get_messages",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
