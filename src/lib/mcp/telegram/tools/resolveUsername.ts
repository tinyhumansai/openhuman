/**
 * Resolve Username tool - Resolve a username to a user or chat ID
 *
 * Uses Telegram's contacts.ResolveUsername API for server-side resolution.
 * Falls back to cached chat lookup if the API call fails.
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { formatEntity, getChatById } from "../telegramApi";
import { mtprotoService } from "../../../../services/mtprotoService";
import { Api } from "telegram";

export const tool: MCPTool = {
  name: "resolve_username",
  description: "Resolve a username to a user or chat ID",
  inputSchema: {
    type: "object",
    properties: {
      username: {
        type: "string",
        description: "Username to resolve (without @)",
      },
    },
    required: ["username"],
  },
};

export async function resolveUsername(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const raw = typeof args.username === "string" ? args.username : "";
    if (!raw) {
      return {
        content: [{ type: "text", text: "username is required" }],
        isError: true,
      };
    }
    const username = raw.startsWith("@") ? raw.slice(1) : raw;

    // Try server-side resolution via Telegram API
    try {
      const client = mtprotoService.getClient();

      const result = await mtprotoService.withFloodWaitHandling(async () => {
        return client.invoke(
          new Api.contacts.ResolveUsername({ username }),
        );
      });

      if (result && "peer" in result) {
        const peer = result.peer;
        const peerType =
          peer.className === "PeerUser"
            ? "user"
            : peer.className === "PeerChannel"
              ? "channel"
              : peer.className === "PeerChat"
                ? "chat"
                : "unknown";

        // Extract ID from peer
        const peerId =
          "userId" in peer
            ? String(peer.userId)
            : "channelId" in peer
              ? String(peer.channelId)
              : "chatId" in peer
                ? String(peer.chatId)
                : "unknown";

        // Try to get name from resolved users/chats
        let name = username;
        if ("users" in result && Array.isArray(result.users)) {
          const user = result.users[0] as {
            firstName?: string;
            lastName?: string;
          };
          if (user) {
            name =
              [user.firstName, user.lastName].filter(Boolean).join(" ") ||
              username;
          }
        }
        if ("chats" in result && Array.isArray(result.chats)) {
          const chat = result.chats[0] as { title?: string };
          if (chat?.title) {
            name = chat.title;
          }
        }

        return {
          content: [
            {
              type: "text",
              text: JSON.stringify(
                { id: peerId, name, type: peerType, username },
                undefined,
                2,
              ),
            },
          ],
        };
      }
    } catch {
      // API call failed — fall back to cached lookup below
    }

    // Fallback: look up from cached state
    const lookupKey = `@${username}`;
    const chat = getChatById(lookupKey);
    if (!chat) {
      return {
        content: [
          { type: "text", text: `Username @${username} not found` },
        ],
        isError: true,
      };
    }
    const entity = formatEntity(chat);
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(
            {
              id: entity.id,
              name: entity.name,
              type: entity.type,
              username: entity.username,
            },
            undefined,
            2,
          ),
        },
      ],
    };
  } catch (error) {
    return logAndFormatError(
      "resolve_username",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.SEARCH,
    );
  }
}
