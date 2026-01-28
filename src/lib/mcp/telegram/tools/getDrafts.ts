import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "get_drafts",
  description: "Get all drafts",
  inputSchema: { type: "object", properties: {} },
};

export async function getDrafts(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const client = mtprotoService.getClient();

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(new Api.messages.GetAllDrafts());
    });

    const updates = result as any;
    if (!updates || !updates.updates || updates.updates.length === 0) {
      return { content: [{ type: 'text', text: 'No drafts found.' }] };
    }

    const lines: string[] = [];
    for (const update of updates.updates) {
      if (update.draft && update.draft.message) {
        const peerId = update.peer?.userId ?? update.peer?.chatId ?? update.peer?.channelId ?? '?';
        lines.push('Peer ' + peerId + ': ' + update.draft.message);
      }
    }

    if (lines.length === 0) {
      return { content: [{ type: 'text', text: 'No drafts found.' }] };
    }

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_drafts',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.DRAFT,
    );
  }
}
