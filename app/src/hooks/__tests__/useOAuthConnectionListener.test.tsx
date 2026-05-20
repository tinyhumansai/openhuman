import { renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { store } from '../../store';
import {
  resetChannelConnectionsState,
  setChannelConnectionStatus,
} from '../../store/channelConnectionsSlice';
import { useOAuthConnectionListener } from '../useOAuthConnectionListener';

const wrapper = ({ children }: { children: ReactNode }) => (
  <Provider store={store}>{children}</Provider>
);

const dispatchOAuthSuccess = (toolkit: string, integrationId = 'integration-123') => {
  window.dispatchEvent(new CustomEvent('oauth:success', { detail: { integrationId, toolkit } }));
};

const dispatchOAuthError = (provider: string, errorCode = 'access_denied', message?: string) => {
  window.dispatchEvent(
    new CustomEvent('oauth:error', { detail: { provider, errorCode, message } })
  );
};

describe('useOAuthConnectionListener (#2128)', () => {
  beforeEach(() => {
    store.dispatch(resetChannelConnectionsState());
  });

  afterEach(() => {
    store.dispatch(resetChannelConnectionsState());
  });

  it('transitions matching channel to connected on oauth:success', () => {
    store.dispatch(
      setChannelConnectionStatus({ channel: 'discord', authMode: 'oauth', status: 'connecting' })
    );

    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });
    dispatchOAuthSuccess('discord');

    const connection = store.getState().channelConnections.connections.discord.oauth;
    expect(connection?.status).toBe('connected');
    expect(connection?.lastError).toBeUndefined();
    expect(connection?.capabilities).toEqual(['read', 'write']);
  });

  it('ignores oauth:success for a different channel', () => {
    store.dispatch(
      setChannelConnectionStatus({ channel: 'discord', authMode: 'oauth', status: 'connecting' })
    );

    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });
    dispatchOAuthSuccess('telegram');

    expect(store.getState().channelConnections.connections.discord.oauth?.status).toBe(
      'connecting'
    );
  });

  it('matches toolkit case-insensitively', () => {
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });
    dispatchOAuthSuccess('Discord');

    expect(store.getState().channelConnections.connections.discord.oauth?.status).toBe('connected');
  });

  it('transitions to error on oauth:error and surfaces the message', () => {
    store.dispatch(
      setChannelConnectionStatus({ channel: 'telegram', authMode: 'oauth', status: 'connecting' })
    );

    renderHook(() => useOAuthConnectionListener({ channel: 'telegram', authMode: 'oauth' }), {
      wrapper,
    });
    dispatchOAuthError('telegram', 'access_denied', 'User cancelled');

    const connection = store.getState().channelConnections.connections.telegram.oauth;
    expect(connection?.status).toBe('error');
    expect(connection?.lastError).toBe('User cancelled');
  });

  it('falls back to a generic error message when none is provided', () => {
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });
    dispatchOAuthError('discord', 'unknown_error');

    const connection = store.getState().channelConnections.connections.discord.oauth;
    expect(connection?.status).toBe('error');
    expect(connection?.lastError).toMatch(/OAuth sign-in did not complete/);
  });

  it('ignores oauth:error for a different channel', () => {
    store.dispatch(
      setChannelConnectionStatus({ channel: 'discord', authMode: 'oauth', status: 'connecting' })
    );

    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });
    dispatchOAuthError('telegram', 'access_denied');

    expect(store.getState().channelConnections.connections.discord.oauth?.status).toBe(
      'connecting'
    );
  });

  it('records custom capabilities on success when provided', () => {
    renderHook(
      () =>
        useOAuthConnectionListener({
          channel: 'discord',
          authMode: 'oauth',
          capabilitiesOnSuccess: ['dm'],
        }),
      { wrapper }
    );
    dispatchOAuthSuccess('discord');

    expect(store.getState().channelConnections.connections.discord.oauth?.capabilities).toEqual([
      'dm',
    ]);
  });

  it('unsubscribes on unmount so further events do not mutate state', () => {
    const { unmount } = renderHook(
      () => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }),
      { wrapper }
    );
    unmount();
    dispatchOAuthSuccess('discord');

    // No listener mounted any more — the slice stays at its initial state for
    // discord.oauth (undefined, not connected).
    expect(store.getState().channelConnections.connections.discord.oauth).toBeUndefined();
  });
});
