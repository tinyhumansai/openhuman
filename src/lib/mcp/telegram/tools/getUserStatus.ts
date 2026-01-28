import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: 'get_user_status',
  description: 'Get online status of a user',
  inputSchema: {
    type: 'object',
    properties: {
      user_id: { type: 'string', description: 'User ID' },
    },
    required: ['user_id'],
  },
};

export async function getUserStatus(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const userId = validateId(args.user_id, 'user_id');
    const client = mtprotoService.getClient();

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      const inputUser = await client.getInputEntity(userId);
      return client.invoke(
        new Api.users.GetUsers({ id: [inputUser as Api.TypeInputUser] }),
      );
    });

    if (!result || !Array.isArray(result) || result.length === 0) {
      return { content: [{ type: 'text', text: 'User ' + userId + ' not found.' }], isError: true };
    }

    const user = result[0] as any;
    const name = [user.firstName, user.lastName].filter(Boolean).join(' ') || 'Unknown';
    let statusText = 'unknown';

    if (user.status) {
      const s = user.status;
      if (s.className === 'UserStatusOnline') statusText = 'Online';
      else if (s.className === 'UserStatusOffline')
        statusText = 'Offline (last seen: ' + (s.wasOnline ? new Date(s.wasOnline * 1000).toISOString() : 'unknown') + ')';
      else if (s.className === 'UserStatusRecently') statusText = 'Recently';
      else if (s.className === 'UserStatusLastWeek') statusText = 'Last week';
      else if (s.className === 'UserStatusLastMonth') statusText = 'Last month';
      else statusText = s.className ?? 'unknown';
    }

    return { content: [{ type: 'text', text: name + ' (ID: ' + user.id + '): ' + statusText }] };
  } catch (error) {
    return logAndFormatError(
      'get_user_status',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
