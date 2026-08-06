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
      errorMessageKey: null,
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
      errorMessageKey: null,
      requiresAppDataReset: false,
    });
  });

  it('completeDeepLinkAuthProcessing returns to idle', () => {
    beginDeepLinkAuthProcessing();
    completeDeepLinkAuthProcessing();
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: null,
      errorMessageKey: null,
      requiresAppDataReset: false,
    });
  });

  it('failDeepLinkAuthProcessing surfaces message and resets processing flag', () => {
    beginDeepLinkAuthProcessing();
    failDeepLinkAuthProcessing('token expired');
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: 'token expired',
      errorMessageKey: null,
      requiresAppDataReset: false,
    });
  });

  it('failDeepLinkAuthProcessing carries through the requiresAppDataReset hint', () => {
    failDeepLinkAuthProcessing('cannot decrypt', { requiresAppDataReset: true });
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: 'cannot decrypt',
      errorMessageKey: null,
      requiresAppDataReset: true,
    });
  });

  // Deep-link auth runs outside React and cannot call `useT()`, so failures
  // whose copy is localized hand over an i18n key for the rendering component
  // to resolve. Everything else keeps a literal message.
  it('failDeepLinkAuthProcessing carries an i18n key for localized failures', () => {
    failDeepLinkAuthProcessing('', { messageKey: 'welcome.coreConfigUnreadable' });
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage: '',
      errorMessageKey: 'welcome.coreConfigUnreadable',
      requiresAppDataReset: false,
    });
  });

  it('clears a stale i18n key on the next transition', () => {
    failDeepLinkAuthProcessing('', { messageKey: 'welcome.coreConfigUnreadable' });
    beginDeepLinkAuthProcessing();
    expect(getDeepLinkAuthState().errorMessageKey).toBeNull();

    failDeepLinkAuthProcessing('', { messageKey: 'welcome.coreConfigUnreadable' });
    completeDeepLinkAuthProcessing();
    expect(getDeepLinkAuthState().errorMessageKey).toBeNull();

    // A later literal-message failure must not inherit the previous key.
    failDeepLinkAuthProcessing('', { messageKey: 'welcome.coreConfigUnreadable' });
    failDeepLinkAuthProcessing('Sign-in failed. Please try again.');
    expect(getDeepLinkAuthState().errorMessageKey).toBeNull();
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
      errorMessageKey: null,
      requiresAppDataReset: false,
    });

    act(() => {
      beginDeepLinkAuthProcessing();
    });
    expect(result.current).toEqual({
      isProcessing: true,
      errorMessage: null,
      errorMessageKey: null,
      requiresAppDataReset: false,
    });

    act(() => {
      failDeepLinkAuthProcessing('denied');
    });
    expect(result.current).toEqual({
      isProcessing: false,
      errorMessage: 'denied',
      errorMessageKey: null,
      requiresAppDataReset: false,
    });
  });
});
