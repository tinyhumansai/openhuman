import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'block_user',
  description: 'Block a user',
  inputSchema: {
    type: 'object',
    properties: {
      user_id: { type: 'string', description: 'User ID to block' },
    },
    required: ['user_id'],
  },
};

export async function blockUser(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const userId = validateId(args.user_id, 'user_id');
    const client = mtprotoService.getClient();

    await mtprotoService.withFloodWaitHandling(async () => {
      const inputUser = await client.getInputEntity(userId);
      await client.invoke(
        new Api.contacts.Block({ id: inputUser as Api.TypeInputPeer }),
      );
    });

    return { content: [{ type: 'text', text: 'User ' + userId + ' blocked.' }] };
  } catch (error) {
    return logAndFormatError(
      'block_user',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
