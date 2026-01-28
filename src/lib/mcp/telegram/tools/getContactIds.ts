import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'get_contact_ids',
  description: 'Get IDs of all contacts',
  inputSchema: { type: 'object', properties: {} },
};

export async function getContactIds(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const client = mtprotoService.getClient();

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(new Api.contacts.GetContactIDs({ hash: BigInt(0) }));
    });

    if (!result || !Array.isArray(result) || result.length === 0) {
      return { content: [{ type: 'text', text: 'No contact IDs found.' }] };
    }

    const ids = result.map((c: any) => String(c.userId ?? c));
    return { content: [{ type: 'text', text: ids.length + ' contacts:\n' + ids.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_contact_ids',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
