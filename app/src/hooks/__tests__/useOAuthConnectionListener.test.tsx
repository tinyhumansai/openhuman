import { act, renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import {
  resetChannelConnectionsState,
  setChannelConnectionStatus,
} from '../../store/channelConnectionsSlice';
import {
  beginDeepLinkAuthProcessing,
  completeDeepLinkAuthProcessing,
} from '../../store/deepLinkAuthState';
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

// #4299: recover from a `connecting` badge that no OAuth deep link ever resolves
// (Discord rejected the redirect_uri, user cancelled / closed the browser).
const GRACE_MS = 2_500;
const TIMEOUT_MS = 120_000;

const setConnecting = () =>
  store.dispatch(
    setChannelConnectionStatus({ channel: 'discord', authMode: 'oauth', status: 'connecting' })
  );

const oauthStatus = () => store.getState().channelConnections.connections.discord.oauth?.status;

describe('useOAuthConnectionListener stuck-connecting recovery (#4299)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    store.dispatch(resetChannelConnectionsState());
  });

  afterEach(() => {
    completeDeepLinkAuthProcessing();
    vi.useRealTimers();
    store.dispatch(resetChannelConnectionsState());
  });

  it('flips to error when the user returns (focus) and no deep link arrives', () => {
    setConnecting();
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });

    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    act(() => {
      vi.advanceTimersByTime(GRACE_MS);
    });

    const connection = store.getState().channelConnections.connections.discord.oauth;
    expect(connection?.status).toBe('error');
    expect(connection?.lastError).toMatch(/Couldn't finish connecting Discord/);
  });

  it('flips to error on the visibilitychange path too', () => {
    setConnecting();
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });

    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    act(() => {
      vi.advanceTimersByTime(GRACE_MS);
    });

    expect(oauthStatus()).toBe('error');
  });

  it('does NOT flip when a success deep link lands within the grace window', () => {
    setConnecting();
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });

    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    act(() => {
      vi.advanceTimersByTime(GRACE_MS - 500);
      dispatchOAuthSuccess('discord');
    });
    act(() => {
      vi.advanceTimersByTime(GRACE_MS + TIMEOUT_MS);
    });

    expect(oauthStatus()).toBe('connected');
  });

  it('flips to error after the absolute timeout when the user never returns', () => {
    setConnecting();
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });

    act(() => {
      vi.advanceTimersByTime(TIMEOUT_MS);
    });
    act(() => {
      vi.advanceTimersByTime(GRACE_MS);
    });

    expect(oauthStatus()).toBe('error');
  });

  it('does not arm recovery when the channel is not connecting', () => {
    store.dispatch(
      setChannelConnectionStatus({ channel: 'discord', authMode: 'oauth', status: 'connected' })
    );
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });

    act(() => {
      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(TIMEOUT_MS + GRACE_MS);
    });

    expect(oauthStatus()).toBe('connected');
  });

  it('skips recovery while a deep-link auth round-trip is in flight', () => {
    setConnecting();
    renderHook(() => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }), {
      wrapper,
    });

    beginDeepLinkAuthProcessing();
    act(() => {
      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(GRACE_MS);
    });

    expect(oauthStatus()).toBe('connecting');
  });

  it('clears timers on unmount so a late timeout cannot mutate state', () => {
    setConnecting();
    const { unmount } = renderHook(
      () => useOAuthConnectionListener({ channel: 'discord', authMode: 'oauth' }),
      { wrapper }
    );

    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    unmount();
    act(() => {
      vi.advanceTimersByTime(TIMEOUT_MS + GRACE_MS);
    });

    expect(oauthStatus()).toBe('connecting');
  });
});
