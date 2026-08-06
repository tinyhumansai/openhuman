import type {
  ChannelAuthMode,
  ChannelConnection,
  ChannelConnectionsState,
  ChannelType,
} from '../../types/channels';

const SEND_PRIORITY: ChannelAuthMode[] = ['managed_dm', 'oauth', 'bot_token', 'api_key'];

function isConnected(connection: ChannelConnection | undefined): boolean {
  return connection?.status === 'connected';
}

export function resolvePreferredAuthModeForChannel(
  state: ChannelConnectionsState,
  channel: ChannelType
): ChannelAuthMode | null {
  const channelModes = state.connections[channel];
  if (!channelModes) return null;
  for (const authMode of SEND_PRIORITY) {
    if (isConnected(channelModes[authMode])) {
      return authMode;
    }
  }
  return null;
}
