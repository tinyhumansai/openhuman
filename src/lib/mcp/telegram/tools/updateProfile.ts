import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';
import { optString } from '../args';

export const tool: MCPTool = {
  name: 'update_profile',
  description: 'Update your Telegram profile',
  inputSchema: {
    type: 'object',
    properties: {
      first_name: { type: 'string', description: 'New first name' },
      last_name: { type: 'string', description: 'New last name' },
      about: { type: 'string', description: 'New bio/about text' },
    },
  },
};

export async function updateProfile(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const firstName = optString(args, 'first_name');
    const lastName = optString(args, 'last_name');
    const about = optString(args, 'about');

    if (!firstName && !lastName && !about) {
      return { content: [{ type: 'text', text: 'At least one of first_name, last_name, or about is required.' }], isError: true };
    }

    const client = mtprotoService.getClient();

    await mtprotoService.withFloodWaitHandling(async () => {
      await client.invoke(
        new Api.account.UpdateProfile({
          firstName: firstName ?? undefined,
          lastName: lastName ?? undefined,
          about: about ?? undefined,
        }),
      );
    });

    const updates: string[] = [];
    if (firstName) updates.push('first_name: ' + firstName);
    if (lastName) updates.push('last_name: ' + lastName);
    if (about) updates.push('about: ' + about);

    return { content: [{ type: 'text', text: 'Profile updated: ' + updates.join(', ') }] };
  } catch (error) {
    return logAndFormatError(
      'update_profile',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.PROFILE,
    );
  }
}
