import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "get_bot_info",
  description: "Get bot information in a chat",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
    },
    required: ["chat_id"],
  },
};

export async function getBotInfo(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const botId = validateId(args.chat_id, 'chat_id');
    const client = mtprotoService.getClient();

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      const inputUser = await client.getInputEntity(botId);
      return client.invoke(
        new Api.users.GetFullUser({ id: inputUser as Api.TypeInputUser }),
      );
    });

    const fullUser = (result as any)?.fullUser;
    const user = (result as any)?.users?.[0];

    if (!user) {
      return { content: [{ type: 'text', text: 'Bot not found: ' + botId }], isError: true };
    }

    const name = [user.firstName, user.lastName].filter(Boolean).join(' ') || 'Unknown';
    const lines = [
      'Name: ' + name,
      'Username: @' + (user.username ?? 'N/A'),
      'ID: ' + user.id,
      'Bot: ' + (user.bot ? 'Yes' : 'No'),
      'About: ' + (fullUser?.about ?? 'N/A'),
      'Bot Info Description: ' + (fullUser?.botInfo?.description ?? 'N/A'),
    ];

    if (fullUser?.botInfo?.commands) {
      lines.push('Commands:');
      for (const cmd of fullUser.botInfo.commands) {
        lines.push('  /' + cmd.command + ' - ' + cmd.description);
      }
    }

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_bot_info',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
