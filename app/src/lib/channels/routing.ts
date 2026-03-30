import type {
  ChannelAuthMode,
  ChannelConnection,
  ChannelConnectionsState,
  ChannelType,
  OutboundRoute,
} from '../../types/channels';

const SEND_PRIORITY: ChannelAuthMode[] = ['managed_dm', 'oauth', 'bot_token', 'api_key'];

const ALL_CHANNELS: ChannelType[] = ['telegram', 'discord', 'web'];

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

export function resolveOutboundRoute(
  state: ChannelConnectionsState,
  preferredChannel?: ChannelType
): OutboundRoute | null {
  const channel = preferredChannel ?? state.defaultMessagingChannel;
  const mode = resolvePreferredAuthModeForChannel(state, channel);
  if (mode) {
    return { channel, authMode: mode };
  }

  // Try other channels as fallback.
  for (const fallback of ALL_CHANNELS) {
    if (fallback === channel) continue;
    const fallbackMode = resolvePreferredAuthModeForChannel(state, fallback);
    if (fallbackMode) {
      return { channel: fallback, authMode: fallbackMode };
    }
  }

  return null;
}
