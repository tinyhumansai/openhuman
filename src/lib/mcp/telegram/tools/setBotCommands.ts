import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "set_bot_commands",
  description: "Set bot commands",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      commands: { type: "array", description: "List of commands" },
    },
    required: ["commands"],
  },
};

export async function setBotCommands(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const cmds = Array.isArray(args.commands) ? args.commands : [];
    if (cmds.length === 0) return { content: [{ type: 'text', text: 'commands array is required' }], isError: true };

    const client = mtprotoService.getClient();

    const botCommands = cmds.map((c: any) =>
      new Api.BotCommand({ command: String(c.command ?? ''), description: String(c.description ?? '') }),
    );

    await mtprotoService.withFloodWaitHandling(async () => {
      await client.invoke(
        new Api.bots.SetBotCommands({
          scope: new Api.BotCommandScopeDefault(),
          langCode: '',
          commands: botCommands,
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Bot commands updated: ' + cmds.length + ' commands.' }] };
  } catch (error) {
    return logAndFormatError(
      'set_bot_commands',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.PROFILE,
    );
  }
}
