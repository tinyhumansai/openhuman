/**
 * Get Chat tool - Get detailed information about a specific chat
 *
 * Tries cached Redux state first, falls back to Telegram API when cache misses.
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { formatEntity, getChatById } from "../telegramApi";
import { validateId } from "../../validation";
import { mtprotoService } from "../../../../services/mtprotoService";
import { enforceRateLimit } from "../../rateLimiter";

export const tool: MCPTool = {
  name: "get_chat",
  description: "Get detailed information about a specific chat",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: {
        type: "string",
        description: "The ID or username of the chat",
      },
    },
    required: ["chat_id"],
  },
};

export async function getChat(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, "chat_id");
    const chat = getChatById(chatId);

    if (chat) {
      // Cache hit — return cached data
      return formatChatResult(chat);
    }

    // Cache miss — try Telegram API
    try {
      await enforceRateLimit("__api_fallback_chat");

      const client = mtprotoService.getClient();
      const entity = await mtprotoService.withFloodWaitHandling(async () => {
        return client.getEntity(chatId);
      });

      if (!entity) {
        return {
          content: [{ type: "text", text: `Chat not found: ${chatId}` }],
          isError: true,
        };
      }

      const raw = entity as unknown as Record<string, unknown>;
      const result: string[] = [];

      result.push(`ID: ${raw.id ?? chatId}`);

      // Determine type and title
      const className = raw.className as string | undefined;
      if (className === "User") {
        const name =
          [raw.firstName, raw.lastName].filter(Boolean).join(" ") || "Unknown";
        result.push(`Name: ${name}`);
        result.push(`Type: user`);
        if (raw.username) result.push(`Username: @${raw.username}`);
        if (raw.phone) result.push(`Phone: +${raw.phone}`);
        if (raw.bot) result.push(`Bot: true`);
      } else if (className === "Channel") {
        result.push(`Title: ${raw.title ?? "Unknown"}`);
        result.push(`Type: ${raw.megagroup ? "supergroup" : "channel"}`);
        if (raw.username) result.push(`Username: @${raw.username}`);
        if (raw.participantsCount)
          result.push(`Participants: ${raw.participantsCount}`);
      } else if (className === "Chat") {
        result.push(`Title: ${raw.title ?? "Unknown"}`);
        result.push(`Type: group`);
        if (raw.participantsCount)
          result.push(`Participants: ${raw.participantsCount}`);
      } else {
        result.push(`Title: ${raw.title ?? raw.firstName ?? "Unknown"}`);
        result.push(`Type: unknown`);
      }

      return { content: [{ type: "text", text: result.join("\n") }] };
    } catch {
      return {
        content: [{ type: "text", text: `Chat not found: ${chatId}` }],
        isError: true,
      };
    }
  } catch (error) {
    return logAndFormatError(
      "get_chat",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CHAT,
    );
  }
}

function formatChatResult(
  chat: ReturnType<typeof getChatById> & object,
): MCPToolResult {
  const entity = formatEntity(chat);
  const result: string[] = [];

  result.push(`ID: ${entity.id}`);
  result.push(`Title: ${entity.name}`);
  result.push(`Type: ${entity.type}`);
  if (entity.username) result.push(`Username: @${entity.username}`);
  if ("participantsCount" in chat && chat.participantsCount) {
    result.push(`Participants: ${chat.participantsCount}`);
  }
  if ("unreadCount" in chat) {
    result.push(`Unread Messages: ${chat.unreadCount ?? 0}`);
  }

  const lastMsg = (chat as { lastMessage?: { fromName?: string; fromId?: string; date: number; message?: string } }).lastMessage;
  if (lastMsg) {
    const from = lastMsg.fromName ?? lastMsg.fromId ?? "Unknown";
    const date = new Date(lastMsg.date * 1000).toISOString();
    result.push(`Last Message: From ${from} at ${date}`);
    result.push(`Message: ${lastMsg.message || "[Media/No text]"}`);
  }

  return { content: [{ type: "text", text: result.join("\n") }] };
}
