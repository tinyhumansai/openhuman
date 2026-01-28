import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

export const tool: MCPTool = {
  name: "set_profile_photo",
  description: "Set profile photo",
  inputSchema: {
    type: "object",
    properties: {
      file_path: { type: 'string', description: 'Path to the photo file' },
    },
    required: ['file_path'],
  },
};

export async function setProfilePhoto(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  return {
    content: [{ type: 'text', text: 'set_profile_photo requires file upload which is not supported via MCP text interface. Use the Telegram client directly.' }],
    isError: true,
  };
}
