import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "get_sticker_sets",
  description: "Get sticker sets",
  inputSchema: { type: "object", properties: {} },
};

export async function getStickerSets(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const client = mtprotoService.getClient();

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(new Api.messages.GetAllStickers({ hash: BigInt(0) }));
    });

    const sets = (result as any)?.sets;
    if (!sets || !Array.isArray(sets) || sets.length === 0) {
      return { content: [{ type: 'text', text: 'No sticker sets found.' }] };
    }

    const lines = sets.map((s: any) => {
      return 'ID: ' + s.id + ' | ' + (s.title ?? 'Untitled') + ' (' + (s.count ?? 0) + ' stickers)';
    });

    return { content: [{ type: 'text', text: lines.length + ' sticker sets:\n' + lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_sticker_sets',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MEDIA,
    );
  }
}
