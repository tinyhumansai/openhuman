import type {
  BotPermissionCheck,
  ChannelAuthMode,
  ChannelConnectionResult,
  ChannelDefinition,
  ChannelStatusEntry,
  ChannelType,
  DiscordGuild,
  DiscordTextChannel,
} from '../../types/channels';
import { callCoreRpc } from '../coreRpcClient';

interface ConnectChannelPayload {
  authMode: ChannelAuthMode;
  credentials?: Record<string, string>;
}

export interface TelegramLoginStartResult {
  linkToken: string;
  telegramUrl: string;
  botUsername: string;
}

export interface DiscordLinkStartResult {
  linkToken: string;
  instructions: string;
}

export interface DiscordLinkCheckResult {
  linked: boolean;
  details?: Record<string, unknown> | null;
}

export interface TelegramLoginCheckResult {
  linked: boolean;
  details?: Record<string, unknown> | null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

function unwrapCliEnvelope<T>(payload: unknown): T {
  const record = asRecord(payload);
  if (record && 'result' in record && 'logs' in record && Array.isArray(record.logs)) {
    return record.result as T;
  }
  return payload as T;
}

function expectArray<T>(payload: unknown, context: string): T[] {
  const unwrapped = unwrapCliEnvelope<unknown>(payload);
  if (!Array.isArray(unwrapped)) {
    throw new Error(`${context} returned an invalid response shape`);
  }
  return unwrapped as T[];
}

function expectObject<T extends object>(payload: unknown, context: string): T {
  const unwrapped = unwrapCliEnvelope<unknown>(payload);
  const record = asRecord(unwrapped);
  if (!record) {
    throw new Error(`${context} returned an invalid response shape`);
  }
  return record as T;
}

function expectDiscordLinkStart(payload: unknown): DiscordLinkStartResult {
  const record = expectObject<Record<string, unknown>>(payload, 'Discord link start');
  if (typeof record.linkToken !== 'string' || !record.linkToken) {
    throw new Error('Discord link start response missing required string field: linkToken');
  }
  if (typeof record.instructions !== 'string') {
    throw new Error('Discord link start response missing required string field: instructions');
  }
  return { linkToken: record.linkToken, instructions: record.instructions };
}

function expectDiscordLinkComplete(payload: unknown): DiscordLinkCheckResult {
  const record = expectObject<Record<string, unknown>>(payload, 'Discord link complete');
  if (typeof record.linked !== 'boolean') {
    throw new Error('Discord link complete response missing required boolean field: linked');
  }
  const details =
    record.details !== undefined && record.details !== null
      ? (record.details as Record<string, unknown>)
      : null;
  return { linked: record.linked, details };
}

function normalizeConnectResult(payload: unknown): ChannelConnectionResult {
  const record = expectObject<Record<string, unknown>>(payload, 'Channel connect');
  const status = typeof record.status === 'string' ? record.status : '';
  if (!status) {
    throw new Error('Channel connect response missing status');
  }
  return {
    status,
    restart_required: Boolean(record.restart_required),
    auth_action: typeof record.auth_action === 'string' ? record.auth_action : undefined,
    message: typeof record.message === 'string' ? record.message : undefined,
  };
}

function normalizePermissionCheck(payload: unknown): BotPermissionCheck {
  const record = expectObject<Record<string, unknown>>(payload, 'Discord permission check');
  const missing = Array.isArray(record.missing_permissions)
    ? record.missing_permissions.filter((perm): perm is string => typeof perm === 'string')
    : [];
  return {
    can_view_channel: Boolean(record.can_view_channel),
    can_send_messages: Boolean(record.can_send_messages),
    can_read_message_history: Boolean(record.can_read_message_history),
    missing_permissions: missing,
  };
}

export const channelConnectionsApi = {
  /** Fetch all available channel definitions from the backend. */
  listDefinitions: async (): Promise<ChannelDefinition[]> => {
    const result = await callCoreRpc<unknown>({ method: 'openhuman.channels_list', params: {} });
    return expectArray<ChannelDefinition>(result, 'Channel definitions');
  },

  /** Get connection status for one or all channels. */
  listStatus: async (channel?: ChannelType): Promise<ChannelStatusEntry[]> => {
    const params: Record<string, string> = {};
    if (channel) params.channel = channel;
    const result = await callCoreRpc<unknown>({ method: 'openhuman.channels_status', params });
    return expectArray<ChannelStatusEntry>(result, 'Channel status');
  },

  /** Connect a channel with the given auth mode and credentials. */
  connectChannel: async (
    channel: ChannelType,
    payload: ConnectChannelPayload
  ): Promise<ChannelConnectionResult> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_connect',
      params: { channel, authMode: payload.authMode, credentials: payload.credentials ?? {} },
    });
    return normalizeConnectResult(result);
  },

  /** Disconnect a channel for a given auth mode. */
  disconnectChannel: async (channel: ChannelType, authMode: ChannelAuthMode): Promise<void> => {
    await callCoreRpc({ method: 'openhuman.channels_disconnect', params: { channel, authMode } });
  },

  /** Test channel credentials without persisting. */
  testChannel: async (
    channel: ChannelType,
    authMode: ChannelAuthMode,
    credentials: Record<string, string>
  ): Promise<{ success: boolean; message: string }> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_test',
      params: { channel, authMode, credentials },
    });
    return expectObject<{ success: boolean; message: string }>(result, 'Channel test');
  },

  /** Initiate managed Telegram DM login — creates a link token and returns a deep link URL. */
  telegramLoginStart: async (): Promise<TelegramLoginStartResult> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_telegram_login_start',
      params: {},
    });
    return expectObject<TelegramLoginStartResult>(result, 'Telegram login start');
  },

  /** Check whether the Telegram managed DM link has been completed. */
  telegramLoginCheck: async (linkToken: string): Promise<TelegramLoginCheckResult> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_telegram_login_check',
      params: { linkToken },
    });
    return expectObject<TelegramLoginCheckResult>(result, 'Telegram login check');
  },

  /** Initiate Discord managed link — creates a link token the user pastes into Discord as `!start <token>`. */
  discordLinkStart: async (): Promise<DiscordLinkStartResult> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_discord_link_start',
      params: {},
    });
    return expectDiscordLinkStart(result);
  },

  /** Check whether the Discord managed link has been completed. */
  discordLinkCheck: async (linkToken: string): Promise<DiscordLinkCheckResult> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_discord_link_check',
      params: { linkToken },
    });
    return expectDiscordLinkComplete(result);
  },

  /** List Discord servers (guilds) the connected bot is a member of. */
  listDiscordGuilds: async (): Promise<DiscordGuild[]> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_discord_list_guilds',
      params: {},
    });
    return expectArray<DiscordGuild>(result, 'Discord guild list');
  },

  /** List text channels in a Discord server. */
  listDiscordChannels: async (guildId: string): Promise<DiscordTextChannel[]> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_discord_list_channels',
      params: { guildId },
    });
    return expectArray<DiscordTextChannel>(result, 'Discord channel list');
  },

  /** Check bot permissions in a Discord channel. */
  checkDiscordPermissions: async (
    guildId: string,
    channelId: string
  ): Promise<BotPermissionCheck> => {
    const result = await callCoreRpc<unknown>({
      method: 'openhuman.channels_discord_check_permissions',
      params: { guildId, channelId },
    });
    return normalizePermissionCheck(result);
  },

  /** Placeholder for default channel preference sync. */
  updatePreferences: async (defaultMessagingChannel: ChannelType): Promise<void> => {
    void defaultMessagingChannel;
  },
};
