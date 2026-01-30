import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { getChatById, getMessagesWithApiFallback, formatMessage } from "../telegramApi";
import { validateId } from "../../validation";
import { optNumber } from "../args";

export const tool: MCPTool = {
  name: "get_message_context",
  description: "Get context around a specific message",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      message_id: { type: "number", description: "Message ID" },
      limit: {
        type: "number",
        description: "Number of messages before/after",
        default: 5,
      },
    },
    required: ["chat_id", "message_id"],
  },
};

export async function getMessageContext(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, "chat_id");
    const messageId =
      typeof args.message_id === "number" && Number.isInteger(args.message_id)
        ? args.message_id
        : undefined;
    const contextSize = optNumber(args, "limit", 5);

    if (messageId === undefined) {
      return {
        content: [
          { type: "text", text: "message_id must be a positive integer" },
        ],
        isError: true,
      };
    }

    const chat = getChatById(chatId);
    if (!chat) {
      return {
        content: [{ type: "text", text: `Chat not found: ${chatId}` }],
        isError: true,
      };
    }

    const allMessages = await getMessagesWithApiFallback(chatId, 200, 0);
    if (!allMessages || allMessages.length === 0) {
      return {
        content: [{ type: "text", text: "No messages found in this chat." }],
      };
    }

    const targetIndex = allMessages.findIndex(
      (m) => String(m.id) === String(messageId),
    );
    if (targetIndex === -1) {
      return {
        content: [
          {
            type: "text",
            text: `Message ${messageId} not found in cached messages.`,
          },
        ],
        isError: true,
      };
    }

    const start = Math.max(0, targetIndex - contextSize);
    const end = Math.min(allMessages.length, targetIndex + contextSize + 1);
    const contextMessages = allMessages.slice(start, end);

    const lines = contextMessages.map((msg) => {
      const f = formatMessage(msg);
      const from = msg.fromName ?? msg.fromId ?? "Unknown";
      const marker = String(msg.id) === String(messageId) ? " >>> " : "     ";
      return `${marker}ID: ${f.id} | ${from} | ${f.date} | ${f.text || "[Media/No text]"}`;
    });

    return { content: [{ type: "text", text: lines.join("\n") }] };
  } catch (error) {
    return logAndFormatError(
      "get_message_context",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
