/**
 * Search Messages tool - Search for messages in a chat by text
 *
 * Uses Telegram's messages.Search API for server-side full-text search.
 * Falls back to filtering cached messages if the API call fails.
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { optNumber } from "../args";
import { formatMessage, getChatById, getMessages } from "../telegramApi";
import { validateId } from "../../validation";
import { mtprotoService } from "../../../../services/mtprotoService";
import { Api } from "telegram";
import bigInt from "big-integer";
import type { ApiMessage } from "../apiResultTypes";
import { narrow } from "../apiCastHelpers";

export const tool: MCPTool = {
  name: "search_messages",
  description: "Search for messages in a chat by text",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "The chat ID or username" },
      query: { type: "string", description: "Search query" },
      limit: {
        type: "number",
        description: "Maximum number of messages to return",
        default: 20,
      },
    },
    required: ["chat_id", "query"],
  },
};

export async function searchMessages(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, "chat_id");
    const query = typeof args.query === "string" ? args.query : "";
    const limit = optNumber(args, "limit", 20);

    if (!query) {
      return {
        content: [{ type: "text", text: "query is required" }],
        isError: true,
      };
    }

    const chat = getChatById(chatId);
    if (!chat) {
      return {
        content: [{ type: "text", text: `Chat ${chatId} not found` }],
        isError: true,
      };
    }

    // Try server-side search via Telegram API
    try {
      const client = mtprotoService.getClient();
      const entity = chat.username ? chat.username : chat.id;
      const inputPeer = await client.getInputEntity(entity);

      const result = await mtprotoService.withFloodWaitHandling(async () => {
        return client.invoke(
          new Api.messages.Search({
            peer: inputPeer,
            q: query,
            filter: new Api.InputMessagesFilterEmpty(),
            minDate: 0,
            maxDate: 0,
            offsetId: 0,
            addOffset: 0,
            limit,
            maxId: 0,
            minId: 0,
            hash: bigInt(0),
          }),
        );
      });

      if ("messages" in result && Array.isArray(result.messages)) {
        const messages = narrow<ApiMessage[]>(result.messages);

        if (messages.length === 0) {
          return {
            content: [
              { type: "text", text: `No messages matching "${query}" found.` },
            ],
          };
        }

        const lines = messages.map((msg) => {
          const id = msg.id ?? "?";
          const text = msg.message ?? "[Media/No text]";
          const date = msg.date
            ? new Date(msg.date * 1000).toISOString()
            : "unknown";
          return `ID: ${id} | Date: ${date} | ${text}`;
        });

        return { content: [{ type: "text", text: lines.join("\n") }] };
      }
    } catch {
      // API call failed — fall back to cached message search below
    }

    // Fallback: search cached messages locally
    const messages = await getMessages(chatId, Math.min(limit * 3, 100), 0);
    if (!messages || messages.length === 0) {
      return { content: [{ type: "text", text: "No messages found." }] };
    }

    const q = query.toLowerCase();
    const filtered = messages
      .filter((m) => (m.message ?? "").toLowerCase().includes(q))
      .slice(0, limit);

    if (filtered.length === 0) {
      return {
        content: [
          { type: "text", text: `No messages matching "${query}" found.` },
        ],
      };
    }

    const lines = filtered.map((m) => {
      const f = formatMessage(m);
      return `ID: ${f.id} | ${m.fromName ?? m.fromId ?? "Unknown"} | ${f.date} | ${f.text || "[Media]"}`;
    });

    return {
      content: [
        { type: "text", text: `(cached search)\n${lines.join("\n")}` },
      ],
    };
  } catch (error) {
    return logAndFormatError(
      "search_messages",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.SEARCH,
    );
  }
}
