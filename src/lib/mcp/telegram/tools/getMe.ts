/**
 * Get Me tool - Get your own user information
 */

import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";

import { ErrorCategory, logAndFormatError } from "../../errorHandler";
import { formatEntity, getCurrentUserWithApiFallback } from "../telegramApi";

export const tool: MCPTool = {
  name: "get_me",
  description: "Get your own user information",
  inputSchema: {
    type: "object",
    properties: {},
  },
};

export async function getMe(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const user = await getCurrentUserWithApiFallback();
    if (!user) {
      return {
        content: [{ type: "text", text: "User information not available" }],
        isError: true,
      };
    }
    const entity = formatEntity(user);
    const result = {
      id: entity.id,
      name: entity.name,
      type: entity.type,
      ...(entity.username && { username: entity.username }),
      ...(entity.phone && { phone: entity.phone }),
    };
    return {
      content: [{ type: "text", text: JSON.stringify(result, undefined, 2) }],
    };
  } catch (error) {
    return logAndFormatError(
      "get_me",
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.PROFILE,
    );
  }
}
