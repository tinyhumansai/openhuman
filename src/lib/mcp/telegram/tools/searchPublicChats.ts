/**
 * Search Public Chats tool - Search for public chats, channels, or bots
 *
 * Uses Telegram's contacts.Search API for server-side discovery.
 * Falls back to filtering cached chats if the API call fails.
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { formatEntity, searchChats } from "../telegramApi";
import { mtprotoService } from "../../../../services/mtprotoService";
import { Api } from "telegram";

export const tool: MCPTool = {
  name: "search_public_chats",
  description:
    "Search for public chats, channels, or bots by username or title",
  inputSchema: {
    type: "object",
    properties: {
      query: { type: "string", description: "Search query" },
    },
    required: ["query"],
  },
};

export async function searchPublicChats(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const query = typeof args.query === "string" ? args.query : "";
    if (!query) {
      return {
        content: [{ type: "text", text: "query is required" }],
        isError: true,
      };
    }

    // Try server-side search via Telegram API
    try {
      const client = mtprotoService.getClient();

      const result = await mtprotoService.withFloodWaitHandling(async () => {
        return client.invoke(
          new Api.contacts.Search({ q: query, limit: 20 }),
        );
      });

      const entries: Array<{
        id: string;
        name: string;
        type: string;
        username?: string;
      }> = [];

      // Process returned chats
      if ("chats" in result && Array.isArray(result.chats)) {
        for (const chat of result.chats) {
          const c = chat as {
            id: { toString(): string };
            title?: string;
            username?: string;
            megagroup?: boolean;
            broadcast?: boolean;
          };
          const type = c.broadcast
            ? "channel"
            : c.megagroup
              ? "group"
              : "chat";
          entries.push({
            id: String(c.id),
            name: c.title ?? "Unknown",
            type,
            username: c.username,
          });
        }
      }

      // Process returned users
      if ("users" in result && Array.isArray(result.users)) {
        for (const user of result.users) {
          const u = user as {
            id: { toString(): string };
            firstName?: string;
            lastName?: string;
            username?: string;
            bot?: boolean;
          };
          const name =
            [u.firstName, u.lastName].filter(Boolean).join(" ") || "Unknown";
          entries.push({
            id: String(u.id),
            name,
            type: u.bot ? "bot" : "user",
            username: u.username,
          });
        }
      }

      if (entries.length > 0) {
        return {
          content: [
            { type: "text", text: JSON.stringify(entries, undefined, 2) },
          ],
        };
      }
    } catch {
      // API call failed — fall back to cached search below
    }

    // Fallback: search cached chats locally
    const chats = await searchChats(query);
    const results = chats.map(formatEntity);

    if (results.length === 0) {
      return {
        content: [
          {
            type: "text",
            text: `No public chats matching "${query}" found.`,
          },
        ],
      };
    }

    return {
      content: [
        {
          type: "text",
          text: `(cached search)\n${JSON.stringify(results, undefined, 2)}`,
        },
      ],
    };
  } catch (error) {
    return logAndFormatError(
      "search_public_chats",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.SEARCH,
    );
  }
}
