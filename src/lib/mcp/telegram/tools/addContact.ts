import type { MCPTool, MCPToolResult } from '../../types';
import type { TelegramMCPContext } from '../types';
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';
import { optString } from '../args';

export const tool: MCPTool = {
  name: 'add_contact',
  description: 'Add a contact to your Telegram account',
  inputSchema: {
    type: 'object',
    properties: {
      phone: { type: 'string', description: 'Phone number' },
      first_name: { type: 'string', description: 'First name' },
      last_name: { type: 'string', description: 'Last name' },
    },
    required: ['phone', 'first_name'],
  },
};

export async function addContact(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const phone = typeof args.phone === 'string' ? args.phone : '';
    const firstName = typeof args.first_name === 'string' ? args.first_name : '';
    const lastName = optString(args, 'last_name') ?? '';

    if (!phone) return { content: [{ type: 'text', text: 'phone is required' }], isError: true };
    if (!firstName) return { content: [{ type: 'text', text: 'first_name is required' }], isError: true };

    const client = mtprotoService.getClient();

    await mtprotoService.withFloodWaitHandling(async () => {
      await client.invoke(
        new Api.contacts.ImportContacts({
          contacts: [
            new Api.InputPhoneContact({
              clientId: BigInt(0),
              phone,
              firstName,
              lastName,
            }),
          ],
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Contact ' + firstName + ' ' + lastName + ' (' + phone + ') added.' }] };
  } catch (error) {
    return logAndFormatError(
      'add_contact',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.CONTACT,
    );
  }
}
