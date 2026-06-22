import { renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import { store } from '../../store';
import { resetChannelConnectionsState } from '../../store/channelConnectionsSlice';
import type { ChannelStatusEntry } from '../../types/channels';
import { resolveStatusPatch, useChannelDefinitions } from '../useChannelDefinitions';

vi.mock('../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: {
    listDefinitions: vi.fn(),
    listStatus: vi.fn(),
    getDefaultChannel: vi.fn(),
  },
}));

const mockApi = vi.mocked(channelConnectionsApi);

function entry(overrides: Partial<ChannelStatusEntry>): ChannelStatusEntry {
  return {
    channel_id: 'discord',
    auth_mode: 'bot_token',
    connected: false,
    has_credentials: true,
    ...overrides,
  };
}

describe('resolveStatusPatch (issue #3712)', () => {
  it('asserts connected and clears any prior error', () => {
    expect(resolveStatusPatch(entry({ connected: true, error: 'stale' }), 'error')).toEqual({
      status: 'connected',
      lastError: undefined,
    });
  });

  it('surfaces a live listener error with its reason', () => {
    expect(
      resolveStatusPatch(entry({ connected: false, error: 'gateway closed (4004)' }), 'connected')
    ).toEqual({ status: 'error', lastError: 'gateway closed (4004)' });
  });

  it('does not stomp an in-flight connect when not-connected with no error', () => {
    expect(resolveStatusPatch(entry({ connected: false }), 'connecting')).toBeNull();
  });

  it('downgrades a stale connected entry to disconnected', () => {
    expect(resolveStatusPatch(entry({ connected: false }), 'connected')).toEqual({
      status: 'disconnected',
      lastError: undefined,
    });
  });

  it('reports disconnected when there is no prior status', () => {
    expect(resolveStatusPatch(entry({ connected: false }), undefined)).toEqual({
      status: 'disconnected',
      lastError: undefined,
    });
  });
});

describe('useChannelDefinitions loadDefinitions (issue #3794)', () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );

  beforeEach(() => {
    store.dispatch(resetChannelConnectionsState());
    mockApi.listDefinitions.mockResolvedValue([]);
    mockApi.getDefaultChannel.mockResolvedValue('discord');
    mockApi.listStatus.mockResolvedValue([
      { channel_id: 'discord', auth_mode: 'bot_token', connected: true, has_credentials: true },
      // Unknown channel from core must be skipped, not coerced into state (#3794).
      {
        channel_id: 'bogus',
        auth_mode: 'bot_token',
        connected: true,
        has_credentials: true,
      } as ChannelStatusEntry,
    ]);
  });

  afterEach(() => {
    store.dispatch(resetChannelConnectionsState());
    vi.clearAllMocks();
  });

  it('seeds the default channel from core, syncs known channels, and skips unknown ones', async () => {
    const { result } = renderHook(() => useChannelDefinitions(), { wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));

    const state = store.getState().channelConnections;
    // Default channel seeded from the core (source of truth).
    expect(state.defaultMessagingChannel).toBe('discord');
    // Known channel synced as connected.
    expect(state.connections.discord?.bot_token?.status).toBe('connected');
    // Unknown channel_id ignored — never added to state.
    expect((state.connections as Record<string, unknown>).bogus).toBeUndefined();
  });
});
