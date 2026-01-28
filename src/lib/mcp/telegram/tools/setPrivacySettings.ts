import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "set_privacy_settings",
  description: "Set privacy settings",
  inputSchema: {
    type: "object",
    properties: {
      setting: { type: "string", description: "Setting name" },
      value: { type: "string", description: "Value" },
    },
    required: ["setting", "value"],
  },
};

export async function setPrivacySettings(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const keyStr = typeof args.setting === 'string' ? args.setting : '';
    const ruleStr = typeof args.value === 'string' ? args.value : '';

    if (!keyStr) return { content: [{ type: 'text', text: 'setting is required' }], isError: true };
    if (!ruleStr) return { content: [{ type: 'text', text: 'value is required' }], isError: true };

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
      return { content: [{ type: 'text', text: 'Unknown privacy key: ' + keyStr }], isError: true };
    }

    const ruleMap: Record<string, Api.TypeInputPrivacyRule> = {
      allow_all: new Api.InputPrivacyValueAllowAll(),
      allow_contacts: new Api.InputPrivacyValueAllowContacts(),
      disallow_all: new Api.InputPrivacyValueDisallowAll(),
    };

    const rule = ruleMap[ruleStr];
    if (!rule) {
      return { content: [{ type: 'text', text: 'Unknown rule: ' + ruleStr + '. Valid: allow_all, allow_contacts, disallow_all' }], isError: true };
    }

    await mtprotoService.withFloodWaitHandling(async () => {
      await client.invoke(new Api.account.SetPrivacy({ key, rules: [rule] }));
    });

    return { content: [{ type: 'text', text: 'Privacy setting ' + keyStr + ' set to ' + ruleStr + '.' }] };
  } catch (error) {
    return logAndFormatError(
      'set_privacy_settings',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.PROFILE,
    );
  }
}
