import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type {
  ChannelAuthMode,
  ChannelConnection,
  ChannelConnectionsState,
  ChannelConnectionStatus,
  ChannelType,
} from '../types/channels';

const SCHEMA_VERSION = 1;

const makeEmptyChannelModes = () => ({
  managed_dm: undefined,
  oauth: undefined,
  bot_token: undefined,
  api_key: undefined,
});

const initialState: ChannelConnectionsState = {
  schemaVersion: SCHEMA_VERSION,
  migrationCompleted: false,
  defaultMessagingChannel: 'telegram',
  connections: {
    telegram: makeEmptyChannelModes(),
    discord: makeEmptyChannelModes(),
    web: makeEmptyChannelModes(),
  },
};

function touchConnection(
  existing: ChannelConnection | undefined,
  patch: Partial<ChannelConnection> & { channel: ChannelType; authMode: ChannelAuthMode }
): ChannelConnection {
  const hasLastError = Object.prototype.hasOwnProperty.call(patch, 'lastError');
  const hasCapabilities = Object.prototype.hasOwnProperty.call(patch, 'capabilities');
  return {
    channel: patch.channel,
    authMode: patch.authMode,
    status: patch.status ?? existing?.status ?? 'disconnected',
    selectedDefault: patch.selectedDefault ?? existing?.selectedDefault ?? false,
    lastError: hasLastError ? patch.lastError : existing?.lastError,
    capabilities: hasCapabilities ? (patch.capabilities ?? []) : (existing?.capabilities ?? []),
    updatedAt: patch.updatedAt ?? new Date().toISOString(),
  };
}

const channelConnectionsSlice = createSlice({
  name: 'channelConnections',
  initialState,
  reducers: {
    completeBreakingMigration(state) {
      if (state.migrationCompleted) return;
      state.connections.telegram = makeEmptyChannelModes();
      state.connections.discord = makeEmptyChannelModes();
      state.connections.web = makeEmptyChannelModes();
      state.defaultMessagingChannel = 'telegram';
      state.migrationCompleted = true;
      state.schemaVersion = SCHEMA_VERSION;
    },

    setDefaultMessagingChannel(state, action: PayloadAction<ChannelType>) {
      state.defaultMessagingChannel = action.payload;
    },

    upsertChannelConnection(
      state,
      action: PayloadAction<{
        channel: ChannelType;
        authMode: ChannelAuthMode;
        patch: Partial<ChannelConnection>;
      }>
    ) {
      const { channel, authMode, patch } = action.payload;
      const existing = state.connections[channel][authMode];
      state.connections[channel][authMode] = touchConnection(existing, {
        channel,
        authMode,
        ...patch,
      });
    },

    setChannelConnectionStatus(
      state,
      action: PayloadAction<{
        channel: ChannelType;
        authMode: ChannelAuthMode;
        status: ChannelConnectionStatus;
        lastError?: string;
      }>
    ) {
      const { channel, authMode, status, lastError } = action.payload;
      const existing = state.connections[channel][authMode];
      state.connections[channel][authMode] = touchConnection(existing, {
        channel,
        authMode,
        status,
        lastError,
      });
    },

    disconnectChannelConnection(
      state,
      action: PayloadAction<{ channel: ChannelType; authMode: ChannelAuthMode }>
    ) {
      const { channel, authMode } = action.payload;
      state.connections[channel][authMode] = touchConnection(state.connections[channel][authMode], {
        channel,
        authMode,
        status: 'disconnected',
        lastError: undefined,
      });
    },

    resetChannelConnectionsState() {
      return initialState;
    },
  },
});

export const {
  completeBreakingMigration,
  setDefaultMessagingChannel,
  upsertChannelConnection,
  setChannelConnectionStatus,
  disconnectChannelConnection,
  resetChannelConnectionsState,
} = channelConnectionsSlice.actions;

export default channelConnectionsSlice.reducer;
