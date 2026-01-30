/**
 * Reminders Skill
 *
 * Provides tools for creating and managing time-based reminders.
 * Reminders are stored in memory and trigger a message send when due.
 */

import type { MCPTool, MCPToolResult } from "../types";
import type { ExtraTool } from "./types";

// ---------------------------------------------------------------------------
// In-memory reminder store
// ---------------------------------------------------------------------------

interface Reminder {
  id: string;
  message: string;
  chatId?: string;
  dueAt: number; // Unix timestamp (seconds)
  createdAt: number;
  fired: boolean;
}

const reminders: Map<string, Reminder> = new Map();
let nextId = 1;

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

const createReminderTool: MCPTool = {
  name: "create_reminder",
  description:
    "Create a reminder that will fire after a specified delay. Optionally associate with a chat.",
  inputSchema: {
    type: "object",
    properties: {
      message: {
        type: "string",
        description: "Reminder message / what to remember",
      },
      delay_seconds: {
        type: "number",
        description: "Delay in seconds from now until the reminder fires",
      },
      chat_id: {
        type: "string",
        description: "Optional chat ID to associate the reminder with",
      },
    },
    required: ["message", "delay_seconds"],
  },
};

const listRemindersTool: MCPTool = {
  name: "list_reminders",
  description: "List all active (not yet fired) reminders.",
  inputSchema: {
    type: "object",
    properties: {},
  },
};

const cancelReminderTool: MCPTool = {
  name: "cancel_reminder",
  description: "Cancel a reminder by its ID.",
  inputSchema: {
    type: "object",
    properties: {
      reminder_id: {
        type: "string",
        description: "The ID of the reminder to cancel",
      },
    },
    required: ["reminder_id"],
  },
};

const getDueRemindersTool: MCPTool = {
  name: "get_due_reminders",
  description: "Get all reminders that are due (past their scheduled time).",
  inputSchema: {
    type: "object",
    properties: {},
  },
};

const clearAllRemindersTool: MCPTool = {
  name: "clear_all_reminders",
  description: "Clear all reminders (both active and fired).",
  inputSchema: {
    type: "object",
    properties: {},
  },
};

// ---------------------------------------------------------------------------
// Skill registration
// ---------------------------------------------------------------------------

export const REMINDERS_EXTRA_TOOL: ExtraTool = {
  name: "reminders",
  description:
    "Create and manage time-based reminders. Set delays, list active reminders, check due reminders, and cancel.",
  tools: [
    createReminderTool,
    listRemindersTool,
    cancelReminderTool,
    getDueRemindersTool,
    clearAllRemindersTool,
  ],
  readOnlyTools: ["list_reminders", "get_due_reminders"],
  contextPrompt: `You now have access to reminder tools. You can create time-based reminders for the user.
- Reminders are stored in memory and will not persist across app restarts
- Use delay_seconds to specify when the reminder should fire (e.g. 300 for 5 minutes)
- You can optionally associate a reminder with a chat_id`,
};

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

export async function executeRemindersTool(
  toolName: string,
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  switch (toolName) {
    case "create_reminder":
      return executeCreateReminder(args);
    case "list_reminders":
      return executeListReminders();
    case "cancel_reminder":
      return executeCancelReminder(args);
    case "get_due_reminders":
      return executeGetDueReminders();
    case "clear_all_reminders":
      return executeClearAllReminders();
    default:
      return {
        content: [
          { type: "text", text: `Unknown reminders tool: ${toolName}` },
        ],
        isError: true,
      };
  }
}

// ---------------------------------------------------------------------------
// Individual executors
// ---------------------------------------------------------------------------

function executeCreateReminder(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const message = args.message as string;
  const delaySeconds = args.delay_seconds as number;
  const chatId = args.chat_id as string | undefined;

  if (!message || typeof delaySeconds !== "number" || delaySeconds <= 0) {
    return Promise.resolve({
      content: [
        {
          type: "text",
          text: "message (string) and delay_seconds (positive number) are required",
        },
      ],
      isError: true,
    });
  }

  const now = Math.floor(Date.now() / 1000);
  const id = String(nextId++);
  const reminder: Reminder = {
    id,
    message,
    chatId,
    dueAt: now + delaySeconds,
    createdAt: now,
    fired: false,
  };
  reminders.set(id, reminder);

  return Promise.resolve({
    content: [
      {
        type: "text",
        text: `Reminder #${id} created. Will fire in ${delaySeconds}s.${chatId ? ` (chat: ${chatId})` : ""}`,
      },
    ],
    isError: false,
  });
}

function executeListReminders(): Promise<MCPToolResult> {
  const active = [...reminders.values()].filter((r) => !r.fired);
  if (active.length === 0) {
    return Promise.resolve({
      content: [{ type: "text", text: "No active reminders." }],
      isError: false,
    });
  }

  const now = Math.floor(Date.now() / 1000);
  const lines = active.map((r) => {
    const remaining = r.dueAt - now;
    const status = remaining > 0 ? `in ${remaining}s` : "OVERDUE";
    return `#${r.id}: "${r.message}" — ${status}${r.chatId ? ` (chat: ${r.chatId})` : ""}`;
  });

  return Promise.resolve({
    content: [
      {
        type: "text",
        text: `Active reminders (${active.length}):\n${lines.join("\n")}`,
      },
    ],
    isError: false,
  });
}

function executeCancelReminder(
  args: Record<string, unknown>,
): Promise<MCPToolResult> {
  const reminderId = args.reminder_id as string;
  if (!reminderId) {
    return Promise.resolve({
      content: [{ type: "text", text: "reminder_id is required" }],
      isError: true,
    });
  }

  if (!reminders.has(reminderId)) {
    return Promise.resolve({
      content: [
        { type: "text", text: `Reminder #${reminderId} not found.` },
      ],
      isError: true,
    });
  }

  reminders.delete(reminderId);
  return Promise.resolve({
    content: [
      { type: "text", text: `Reminder #${reminderId} cancelled.` },
    ],
    isError: false,
  });
}

function executeGetDueReminders(): Promise<MCPToolResult> {
  const now = Math.floor(Date.now() / 1000);
  const due = [...reminders.values()].filter(
    (r) => !r.fired && r.dueAt <= now,
  );

  if (due.length === 0) {
    return Promise.resolve({
      content: [{ type: "text", text: "No due reminders." }],
      isError: false,
    });
  }

  // Mark as fired
  for (const r of due) {
    r.fired = true;
  }

  const lines = due.map(
    (r) =>
      `#${r.id}: "${r.message}"${r.chatId ? ` (chat: ${r.chatId})` : ""}`,
  );

  return Promise.resolve({
    content: [
      {
        type: "text",
        text: `Due reminders (${due.length}):\n${lines.join("\n")}`,
      },
    ],
    isError: false,
  });
}

function executeClearAllReminders(): Promise<MCPToolResult> {
  const count = reminders.size;
  reminders.clear();
  nextId = 1;

  return Promise.resolve({
    content: [
      { type: "text", text: `Cleared ${count} reminder(s).` },
    ],
    isError: false,
  });
}
