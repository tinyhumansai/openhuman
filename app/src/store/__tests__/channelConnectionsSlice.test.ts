import { describe, expect, it } from 'vitest';

import reducer, {
  completeBreakingMigration,
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
