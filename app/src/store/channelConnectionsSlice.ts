import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type {
  ChannelAuthMode,
  ChannelConnection,
  ChannelConnectionsState,
  ChannelConnectionStatus,
  ChannelType,
} from '../types/channels';
import { resetUserScopedState } from './resetActions';

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
    // Required by `ChannelType` after #2048 widened the union. Empty
    // entries keep the `Record<ChannelType, …>` total — runtime state
    // populates them when the user wires up credentials.
    lark: makeEmptyChannelModes(),
    dingtalk: makeEmptyChannelModes(),
    // MCP Servers tab is a virtual channel — no auth-mode connections,
    // but must be present to satisfy Record<ChannelType, …>.
    mcp: makeEmptyChannelModes(),
    yuanbao: makeEmptyChannelModes(),
  },
};

// Lazy-init a channel's mode bucket on first write. Protects writes against
// rehydrated state from older app versions (or any channel added after a user
// first persisted state) where `state.connections[channel]` is `undefined`
// because redux-persist's default `autoMergeLevel1` reconciler does not deep-
// merge into `connections`.
function ensureChannelModes(state: ChannelConnectionsState, channel: ChannelType): void {
  if (!state.connections[channel]) {
    state.connections[channel] = makeEmptyChannelModes();
  }
}

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
      // After #2048 widened ChannelType, redux-persist rehydrated states
      // from before the channels existed wouldn't have these keys; without
      // explicit initialisation here, the first `upsertChannelConnection`
      // for either channel would crash on `state.connections[channel]`
      // being undefined. Pin them by default so the migration is total.
      state.connections.yuanbao = makeEmptyChannelModes();
      state.connections.lark = makeEmptyChannelModes();
      state.connections.dingtalk = makeEmptyChannelModes();
      // MCP virtual channel must be present in persisted states migrated from
      // before PR #2276 or the Record<ChannelType,…> shape is incomplete.
      state.connections.mcp = makeEmptyChannelModes();
      state.connections.yuanbao = makeEmptyChannelModes();
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
      ensureChannelModes(state, channel);
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
      ensureChannelModes(state, channel);
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
      ensureChannelModes(state, channel);
      state.connections[channel][authMode] = touchConnection(state.connections[channel][authMode], {
        channel,
        authMode,
        status: 'disconnected',
        lastError: undefined,
      });
    },

    /**
     * Cancel any sibling auth modes on the same channel that are still in
     * the `connecting` state, except the one explicitly started. Fixes #2128
     * where starting a second OAuth method on a channel left the previous
     * method's badge pinned at `Connecting` forever. Cancelled rows transition
     * to `disconnected` (not `error`) so the UI doesn't surface a misleading
     * failure message — the user explicitly switched methods.
     */
    clearOtherPendingForChannel(
      state,
      action: PayloadAction<{ channel: ChannelType; exceptAuthMode: ChannelAuthMode }>
    ) {
      const { channel, exceptAuthMode } = action.payload;
      const modes = state.connections[channel];
      if (!modes) return;
      for (const mode of Object.keys(modes) as ChannelAuthMode[]) {
        if (mode === exceptAuthMode) continue;
        const existing = modes[mode];
        if (existing?.status !== 'connecting') continue;
        modes[mode] = touchConnection(existing, {
          channel,
          authMode: mode,
          status: 'disconnected',
          lastError: undefined,
        });
      }
    },

    resetChannelConnectionsState() {
      return initialState;
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const {
  completeBreakingMigration,
  setDefaultMessagingChannel,
  upsertChannelConnection,
  setChannelConnectionStatus,
  disconnectChannelConnection,
  clearOtherPendingForChannel,
  resetChannelConnectionsState,
} = channelConnectionsSlice.actions;

export default channelConnectionsSlice.reducer;
