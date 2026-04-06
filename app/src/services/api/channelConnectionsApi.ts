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

export interface TelegramLoginCheckResult {
  linked: boolean;
  details?: Record<string, unknown> | null;
}

export const channelConnectionsApi = {
  /** Fetch all available channel definitions from the backend. */
  listDefinitions: async (): Promise<ChannelDefinition[]> => {
    const result = await callCoreRpc<ChannelDefinition[]>({
      method: 'openhuman.channels_list',
      params: {},
    });
    return result;
  },

  /** Get connection status for one or all channels. */
  listStatus: async (channel?: ChannelType): Promise<ChannelStatusEntry[]> => {
    const params: Record<string, string> = {};
    if (channel) params.channel = channel;
    const result = await callCoreRpc<ChannelStatusEntry[]>({
      method: 'openhuman.channels_status',
      params,
    });
    return result;
  },

  /** Connect a channel with the given auth mode and credentials. */
  connectChannel: async (
    channel: ChannelType,
    payload: ConnectChannelPayload
  ): Promise<ChannelConnectionResult> => {
    const result = await callCoreRpc<ChannelConnectionResult>({
      method: 'openhuman.channels_connect',
      params: { channel, authMode: payload.authMode, credentials: payload.credentials ?? {} },
    });
    return result;
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
    const result = await callCoreRpc<{ success: boolean; message: string }>({
      method: 'openhuman.channels_test',
      params: { channel, authMode, credentials },
    });
    return result;
  },

  /** Initiate managed Telegram DM login — creates a link token and returns a deep link URL. */
  telegramLoginStart: async (): Promise<TelegramLoginStartResult> => {
    const result = await callCoreRpc<TelegramLoginStartResult>({
      method: 'openhuman.channels_telegram_login_start',
      params: {},
    });
    return result;
  },

  /** Check whether the Telegram managed DM link has been completed. */
  telegramLoginCheck: async (linkToken: string): Promise<TelegramLoginCheckResult> => {
    const result = await callCoreRpc<TelegramLoginCheckResult>({
      method: 'openhuman.channels_telegram_login_check',
      params: { linkToken },
    });
    return result;
  },

  /** List Discord servers (guilds) the connected bot is a member of. */
  listDiscordGuilds: async (): Promise<DiscordGuild[]> => {
    return callCoreRpc<DiscordGuild[]>({
      method: 'openhuman.channels_discord_list_guilds',
      params: {},
    });
  },

  /** List text channels in a Discord server. */
  listDiscordChannels: async (guildId: string): Promise<DiscordTextChannel[]> => {
    return callCoreRpc<DiscordTextChannel[]>({
      method: 'openhuman.channels_discord_list_channels',
      params: { guildId },
    });
  },

  /** Check bot permissions in a Discord channel. */
  checkDiscordPermissions: async (
    guildId: string,
    channelId: string
  ): Promise<BotPermissionCheck> => {
    return callCoreRpc<BotPermissionCheck>({
      method: 'openhuman.channels_discord_check_permissions',
      params: { guildId, channelId },
    });
  },

  /** Placeholder for default channel preference sync. */
  updatePreferences: async (defaultMessagingChannel: ChannelType): Promise<void> => {
    void defaultMessagingChannel;
  },
};
