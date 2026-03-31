import type {
  ChannelAuthMode,
  ChannelConnectionResult,
  ChannelDefinition,
  ChannelStatusEntry,
  ChannelType,
} from '../../types/channels';
import { callCoreRpc } from '../coreRpcClient';

interface ConnectChannelPayload {
  authMode: ChannelAuthMode;
  credentials?: Record<string, string>;
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

  /** Placeholder for default channel preference sync. */
  updatePreferences: async (defaultMessagingChannel: ChannelType): Promise<void> => {
    void defaultMessagingChannel;
  },
};
