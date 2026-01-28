import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "get_privacy_settings",
  description: "Get privacy settings",
  inputSchema: {
    type: "object",
    properties: {
      key: { type: 'string', description: 'Privacy key: phone_number, last_seen, profile_photo, forwards, phone_call, chat_invite', default: 'last_seen' },
    },
  },
};

export async function getPrivacySettings(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const keyStr = typeof args.key === 'string' ? args.key : 'last_seen';
    const client = mtprotoService.getClient();

    const keyMap: Record<string, Api.TypeInputPrivacyKey> = {
      phone_number: new Api.InputPrivacyKeyPhoneNumber(),
      last_seen: new Api.InputPrivacyKeyStatusTimestamp(),
      profile_photo: new Api.InputPrivacyKeyProfilePhoto(),
      forwards: new Api.InputPrivacyKeyForwards(),
      phone_call: new Api.InputPrivacyKeyPhoneCall(),
      chat_invite: new Api.InputPrivacyKeyChatInvite(),
    };

    const key = keyMap[keyStr];
    if (!key) {
      return { content: [{ type: 'text', text: 'Unknown privacy key: ' + keyStr + '. Valid keys: ' + Object.keys(keyMap).join(', ') }], isError: true };
    }

    const result = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(new Api.account.GetPrivacy({ key }));
    });

    const rules = (result as any)?.rules;
    if (!rules || !Array.isArray(rules)) {
      return { content: [{ type: 'text', text: 'No privacy rules found for ' + keyStr + '.' }] };
    }

    const lines = rules.map((r: any) => r.className ?? 'Unknown rule');
    return { content: [{ type: 'text', text: 'Privacy settings for ' + keyStr + ':\n' + lines.join('\n') }] };
  } catch (error) {
    return logAndFormatError(
      'get_privacy_settings',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.PROFILE,
    );
  }
}
