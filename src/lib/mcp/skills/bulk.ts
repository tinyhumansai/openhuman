/**
 * Bulk Operations Skill
 *
 * Provides batch tools for operating on multiple chats/messages at once.
 * Each operation respects per-item delays to avoid FLOOD_WAIT.
 */

import type { MCPTool, MCPToolResult } from "../types";
import type { ExtraTool } from "./types";
import * as telegramApi from "../telegram/telegramApi";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BATCH_DELAY_MS = 1000;

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

const bulkSendMessageTool: MCPTool = {
  name: "bulk_send_message",
  description:
    "Send the same message to multiple chats. Provide an array of chat IDs and a message.",
  inputSchema: {
    type: "object",
    properties: {
      chat_ids: {
        type: "array",
        items: { type: "string" },
        description: "Array of chat IDs to send the message to",
      },
      message: {
        type: "string",
        description: "Message text to send",
      },
    },
    required: ["chat_ids", "message"],
  },
};

const bulkArchiveChatsTool: MCPTool = {
  name: "bulk_archive_chats",
  description: "Archive multiple chats at once. Provide an array of chat IDs.",
  inputSchema: {
    type: "object",
    properties: {
      chat_ids: {
        type: "array",
        items: { type: "string" },
        description: "Array of chat IDs to archive",
      },
    },
    required: ["chat_ids"],
  },
};

const bulkMarkAsReadTool: MCPTool = {
  name: "bulk_mark_as_read",
  description:
    "Mark messages as read in multiple chats at once. Provide an array of chat IDs.",
  inputSchema: {
    type: "object",
    properties: {
      chat_ids: {
        type: "array",
        items: { type: "string" },
        description: "Array of chat IDs to mark as read",
      },
    },
    required: ["chat_ids"],
  },
};

const bulkDeleteMessagesTool: MCPTool = {
  name: "bulk_delete_messages",
  description:
    "Delete multiple messages from a chat. Provide a chat ID and array of message IDs.",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: {
        type: "string",
        description: "Chat ID to delete messages from",
      },
      message_ids: {
        type: "array",
        items: { type: "string" },
        description: "Array of message IDs to delete",
      },
    },
    required: ["chat_id", "message_ids"],
  },
};

const bulkForwardMessagesTool: MCPTool = {
  name: "bulk_forward_messages",
  description:
    "Forward multiple messages from one chat to another. Provide source chat, message IDs, and target chat.",
  inputSchema: {
    type: "object",
    properties: {
      from_chat_id: {
        type: "string",
        description: "Source chat ID",
      },
      message_ids: {
        type: "array",
        items: { type: "string" },
        description: "Array of message IDs to forward",
      },
      to_chat_id: {
        type: "string",
        description: "Target chat ID to forward messages to",
      },
    },
    required: ["from_chat_id", "message_ids", "to_chat_id"],
  },
};

// ---------------------------------------------------------------------------
// Skill registration
// ---------------------------------------------------------------------------

export const BULK_EXTRA_TOOL: ExtraTool = {
  name: "bulk",
  description:
    "Batch operations for sending, archiving, deleting, forwarding, and marking messages across multiple chats.",
  tools: [
    bulkSendMessageTool,
    bulkArchiveChatsTool,
    bulkMarkAsReadTool,
    bulkDeleteMessagesTool,
    bulkForwardMessagesTool,
  ],
  readOnlyTools: [],
  contextPrompt: `You now have access to bulk operation tools. These allow batch processing across multiple chats/messages.
IMPORTANT: Bulk operations execute sequentially with delays between items to avoid rate limits.
- Each item has a ~1s delay between operations
- If a rate limit error occurs, the batch stops and returns partial results
- Always confirm with the user before executing bulk operations that modify data`,
};

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

export async function executeBulkTool(
  toolName: string,
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  switch (toolName) {
    case "bulk_send_message":
      return executeBulkSendMessage(args);
    case "bulk_archive_chats":
      return executeBulkArchiveChats(args);
    case "bulk_mark_as_read":
      return executeBulkMarkAsRead(args);
    case "bulk_delete_messages":
      return executeBulkDeleteMessages(args);
    case "bulk_forward_messages":
      return executeBulkForwardMessages(args);
    default:
      return {
        content: [{ type: "text", text: `Unknown bulk tool: ${toolName}` }],
        isError: true,
      };
  }
}

// ---------------------------------------------------------------------------
// Individual executors
// ---------------------------------------------------------------------------

async function executeBulkSendMessage(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const chatIds = args.chat_ids as string[];
  const message = args.message as string;

  if (!chatIds?.length || !message) {
    return {
      content: [
        { type: "text", text: "chat_ids (array) and message (string) are required" },
      ],
      isError: true,
    };
  }

  let successCount = 0;
  const errors: string[] = [];

  for (const chatId of chatIds) {
    try {
      await telegramApi.sendMessage(chatId, message);
      successCount++;
      await sleep(BATCH_DELAY_MS);
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      errors.push(`${chatId}: ${msg}`);
      if (isRateLimitError(msg)) break;
    }
  }

  return {
    content: [
      {
        type: "text",
        text: `Sent to ${successCount}/${chatIds.length} chats.${
          errors.length ? `\nErrors:\n${errors.join("\n")}` : ""
        }`,
      },
    ],
    isError: errors.length > 0 && successCount === 0,
  };
}

async function executeBulkArchiveChats(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const chatIds = args.chat_ids as string[];
  if (!chatIds?.length) {
    return {
      content: [{ type: "text", text: "chat_ids (array) is required" }],
      isError: true,
    };
  }

  // Archive is not directly available in telegramApi yet — return planned result
  return {
    content: [
      {
        type: "text",
        text: `Bulk archive planned for ${chatIds.length} chats. Archive API integration pending.`,
      },
    ],
    isError: false,
  };
}

async function executeBulkMarkAsRead(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const chatIds = args.chat_ids as string[];
  if (!chatIds?.length) {
    return {
      content: [{ type: "text", text: "chat_ids (array) is required" }],
      isError: true,
    };
  }

  return {
    content: [
      {
        type: "text",
        text: `Bulk mark-as-read planned for ${chatIds.length} chats. Mark-as-read API integration pending.`,
      },
    ],
    isError: false,
  };
}

async function executeBulkDeleteMessages(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const chatId = args.chat_id as string;
  const messageIds = args.message_ids as string[];

  if (!chatId || !messageIds?.length) {
    return {
      content: [
        {
          type: "text",
          text: "chat_id (string) and message_ids (array) are required",
        },
      ],
      isError: true,
    };
  }

  return {
    content: [
      {
        type: "text",
        text: `Bulk delete planned for ${messageIds.length} messages in chat ${chatId}. Delete API integration pending.`,
      },
    ],
    isError: false,
  };
}

async function executeBulkForwardMessages(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const fromChatId = args.from_chat_id as string;
  const messageIds = args.message_ids as string[];
  const toChatId = args.to_chat_id as string;

  if (!fromChatId || !messageIds?.length || !toChatId) {
    return {
      content: [
        {
          type: "text",
          text: "from_chat_id, message_ids (array), and to_chat_id are required",
        },
      ],
      isError: true,
    };
  }

  return {
    content: [
      {
        type: "text",
        text: `Bulk forward planned: ${messageIds.length} messages from ${fromChatId} to ${toChatId}. Forward API integration pending.`,
      },
    ],
    isError: false,
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isRateLimitError(message: string): boolean {
  return (
    message.includes("FLOOD_WAIT") ||
    message.includes("SLOWMODE_WAIT") ||
    message.includes("Rate limit")
  );
}
