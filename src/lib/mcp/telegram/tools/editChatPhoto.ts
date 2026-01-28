import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

export const tool: MCPTool = {
  name: "edit_chat_photo",
  description: "Edit chat photo",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      file_path: { type: 'string', description: 'Path to the photo file' },
    },
    required: ["chat_id", 'file_path'],
  },
};

export async function editChatPhoto(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  return {
    content: [{ type: 'text', text: 'edit_chat_photo requires file upload which is not supported via MCP text interface. Use the Telegram client directly.' }],
    isError: true,
  };
}
