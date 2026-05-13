import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  beginDeepLinkAuthProcessing,
  completeDeepLinkAuthProcessing,
  failDeepLinkAuthProcessing,
  getDeepLinkAuthState,
  subscribeDeepLinkAuthState,
  useDeepLinkAuthState,
} from '../deepLinkAuthState';

/**
 * Reset module-level state between tests by calling complete() (the default/idle state)
 * before each test's assertions. The ad-hoc store persists across tests.
 */
afterEach(() => {
  completeDeepLinkAuthProcessing();
});

describe('deepLinkAuthState transitions', () => {
  it('starts idle with no error message', () => {
    completeDeepLinkAuthProcessing();
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
  });

  it('beginDeepLinkAuthProcessing flips isProcessing true and clears prior error', () => {
    failDeepLinkAuthProcessing('prior failure');
    expect(getDeepLinkAuthState().errorMessage).toBe('prior failure');

    beginDeepLinkAuthProcessing();
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: true,
      errorMessage: null,
      requiresAppDataReset: false,
    });
  });

  it('completeDeepLinkAuthProcessing returns to idle', () => {
    beginDeepLinkAuthProcessing();
    completeDeepLinkAuthProcessing();
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
  });

  it('failDeepLinkAuthProcessing surfaces message and resets processing flag', () => {
    beginDeepLinkAuthProcessing();
    failDeepLinkAuthProcessing('token expired');
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: 'token expired',
      requiresAppDataReset: false,
    });
  });

  it('failDeepLinkAuthProcessing carries through the requiresAppDataReset hint', () => {
    failDeepLinkAuthProcessing('cannot decrypt', { requiresAppDataReset: true });
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: 'cannot decrypt',
      requiresAppDataReset: true,
    });
  });
});

describe('deepLinkAuthState subscribers', () => {
  it('notifies subscribers on every transition', () => {
    const listener = vi.fn();
    const unsubscribe = subscribeDeepLinkAuthState(listener);

    beginDeepLinkAuthProcessing();
    failDeepLinkAuthProcessing('boom');
    completeDeepLinkAuthProcessing();

    expect(listener).toHaveBeenCalledTimes(3);
    unsubscribe();
  });

  it('stops notifying after unsubscribe', () => {
    const listener = vi.fn();
    const unsubscribe = subscribeDeepLinkAuthState(listener);
    beginDeepLinkAuthProcessing();
    expect(listener).toHaveBeenCalledTimes(1);

    unsubscribe();
    completeDeepLinkAuthProcessing();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('supports multiple independent subscribers', () => {
    const a = vi.fn();
    const b = vi.fn();
    const offA = subscribeDeepLinkAuthState(a);
    const offB = subscribeDeepLinkAuthState(b);

    beginDeepLinkAuthProcessing();
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);

    offA();
    failDeepLinkAuthProcessing('oops');
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(2);

    offB();
  });
});

describe('useDeepLinkAuthState hook', () => {
  it('re-renders when state changes', () => {
    completeDeepLinkAuthProcessing();
    const { result } = renderHook(() => useDeepLinkAuthState());
    expect(result.current).toEqual({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });

    act(() => {
      beginDeepLinkAuthProcessing();
    });
    expect(result.current).toEqual({
      isProcessing: true,
      errorMessage: null,
      requiresAppDataReset: false,
    });

    act(() => {
      failDeepLinkAuthProcessing('denied');
    });
    expect(result.current).toEqual({
      isProcessing: false,
      errorMessage: 'denied',
      requiresAppDataReset: false,
    });
  });
});
