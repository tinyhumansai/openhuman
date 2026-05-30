import { describe, expect, it } from 'vitest';

import reducer, {
  clearOtherPendingForChannel,
  completeBreakingMigration,
  setChannelConnectionStatus,
  setDefaultMessagingChannel,
  upsertChannelConnection,
} from '../channelConnectionsSlice';

describe('channelConnectionsSlice', () => {
  it('completes one-time breaking migration', () => {
    const state = reducer(undefined, completeBreakingMigration());
    expect(state.migrationCompleted).toBe(true);
    expect(state.defaultMessagingChannel).toBe('telegram');
    // Migration must reset every channel in ChannelType so subsequent
    // upsert/setStatus/disconnect actions never crash on `state.connections
    // [channel]` being undefined for users rehydrating persisted state
    // from before #2048 added lark + dingtalk. See CoderRabbit review on
    // PR #2083.
    expect(state.connections.telegram).toBeDefined();
    expect(state.connections.discord).toBeDefined();
    expect(state.connections.web).toBeDefined();
    expect(state.connections.lark).toBeDefined();
    expect(state.connections.dingtalk).toBeDefined();
  });

  it('upsert on a newly-introduced channel does not crash after migration (#2083)', () => {
    // Regression for the persisted-state crash CoderRabbit flagged:
    // before this fix, an old user who had `migrationCompleted: true` in
    // redux-persist but no `connections.lark` key would crash on the
    // first call to upsertChannelConnection for lark.
    const migrated = reducer(undefined, completeBreakingMigration());
    const next = reducer(
      migrated,
      upsertChannelConnection({
        channel: 'lark',
        authMode: 'api_key',
        patch: { status: 'connected', capabilities: ['send_text'] },
      })
    );
    expect(next.connections.lark.api_key?.status).toBe('connected');
    expect(next.connections.lark.api_key?.capabilities).toEqual(['send_text']);

    const next2 = reducer(
      migrated,
      upsertChannelConnection({
        channel: 'dingtalk',
        authMode: 'api_key',
        patch: { status: 'connected', capabilities: ['send_text'] },
      })
    );
    expect(next2.connections.dingtalk.api_key?.status).toBe('connected');
  });

  it('sets default messaging channel', () => {
    const state = reducer(undefined, setDefaultMessagingChannel('discord'));
    expect(state.defaultMessagingChannel).toBe('discord');
  });

  it('upserts channel connection', () => {
    const state = reducer(
      undefined,
      upsertChannelConnection({
        channel: 'telegram',
        authMode: 'managed_dm',
        patch: { status: 'connected', capabilities: ['dm'] },
      })
    );

    expect(state.connections.telegram.managed_dm?.status).toBe('connected');
    expect(state.connections.telegram.managed_dm?.capabilities).toEqual(['dm']);
  });

  describe('clearOtherPendingForChannel (#2128)', () => {
    it('cancels sibling auth modes stuck in connecting', () => {
      const migrated = reducer(undefined, completeBreakingMigration());
      const withTwoPending = [
        upsertChannelConnection({
          channel: 'discord',
          authMode: 'oauth',
          patch: { status: 'connecting' },
        }),
        upsertChannelConnection({
          channel: 'discord',
          authMode: 'managed_dm',
          patch: { status: 'connecting' },
        }),
      ].reduce(reducer, migrated);

      const cleared = reducer(
        withTwoPending,
        clearOtherPendingForChannel({ channel: 'discord', exceptAuthMode: 'managed_dm' })
      );

      expect(cleared.connections.discord.managed_dm?.status).toBe('connecting');
      expect(cleared.connections.discord.oauth?.status).toBe('disconnected');
      expect(cleared.connections.discord.oauth?.lastError).toBeUndefined();
    });

    it('leaves connected and error sibling rows untouched', () => {
      const migrated = reducer(undefined, completeBreakingMigration());
      const mixed = [
        upsertChannelConnection({
          channel: 'discord',
          authMode: 'oauth',
          patch: { status: 'connected', capabilities: ['read', 'write'] },
        }),
        setChannelConnectionStatus({
          channel: 'discord',
          authMode: 'bot_token',
          status: 'error',
          lastError: 'bad token',
        }),
        upsertChannelConnection({
          channel: 'discord',
          authMode: 'managed_dm',
          patch: { status: 'connecting' },
        }),
      ].reduce(reducer, migrated);

      const cleared = reducer(
        mixed,
        clearOtherPendingForChannel({ channel: 'discord', exceptAuthMode: 'managed_dm' })
      );

      // Sibling row that was `connecting` would have flipped, but there's
      // none here — the others are connected/error and must be preserved.
      expect(cleared.connections.discord.oauth?.status).toBe('connected');
      expect(cleared.connections.discord.bot_token?.status).toBe('error');
      expect(cleared.connections.discord.bot_token?.lastError).toBe('bad token');
      expect(cleared.connections.discord.managed_dm?.status).toBe('connecting');
    });

    it('is a no-op when no sibling is pending', () => {
      const migrated = reducer(undefined, completeBreakingMigration());
      const justOne = reducer(
        migrated,
        upsertChannelConnection({
          channel: 'telegram',
          authMode: 'oauth',
          patch: { status: 'connecting' },
        })
      );
      const after = reducer(
        justOne,
        clearOtherPendingForChannel({ channel: 'telegram', exceptAuthMode: 'oauth' })
      );

      expect(after.connections.telegram.oauth?.status).toBe('connecting');
    });

    it('does not affect other channels', () => {
      const migrated = reducer(undefined, completeBreakingMigration());
      const crossChannel = [
        upsertChannelConnection({
          channel: 'discord',
          authMode: 'oauth',
          patch: { status: 'connecting' },
        }),
        upsertChannelConnection({
          channel: 'telegram',
          authMode: 'oauth',
          patch: { status: 'connecting' },
        }),
      ].reduce(reducer, migrated);

      const after = reducer(
        crossChannel,
        clearOtherPendingForChannel({ channel: 'discord', exceptAuthMode: 'bot_token' })
      );

      // discord.oauth was pending and is not the exception → cleared.
      expect(after.connections.discord.oauth?.status).toBe('disconnected');
      // telegram.oauth is a different channel → untouched.
      expect(after.connections.telegram.oauth?.status).toBe('connecting');
    });
  });

  it('lazily initialises a channel modes bucket when persisted state is missing the key', () => {
    // Simulates a rehydrated state from before yuanbao existed: the channel
    // key is absent so `state.connections.yuanbao` is undefined. Without
    // `ensureChannelModes()` the first upsert would crash on
    // `state.connections[channel][authMode]`. See `ensureChannelModes` in
    // channelConnectionsSlice.ts.
    const migrated = reducer(undefined, completeBreakingMigration());
    const partial = {
      ...migrated,
      connections: { ...migrated.connections, yuanbao: undefined as never },
    };

    const next = reducer(
      partial,
      upsertChannelConnection({
        channel: 'yuanbao',
        authMode: 'api_key',
        patch: { status: 'connected' },
      })
    );

    expect(next.connections.yuanbao).toBeDefined();
    expect(next.connections.yuanbao.api_key?.status).toBe('connected');
  });

  it('clears stale lastError when patch explicitly sets undefined', () => {
    const withError = reducer(
      undefined,
      upsertChannelConnection({
        channel: 'discord',
        authMode: 'oauth',
        patch: { status: 'connecting', lastError: 'Initiate oauth flow' },
      })
    );

    const cleared = reducer(
      withError,
      upsertChannelConnection({
        channel: 'discord',
        authMode: 'oauth',
        patch: { status: 'connecting', lastError: undefined },
      })
    );

    expect(cleared.connections.discord.oauth?.lastError).toBeUndefined();
  });
});
