import { isTauri } from '@tauri-apps/api/core';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  completeDeepLinkAuthProcessing,
  getDeepLinkAuthState,
  subscribeDeepLinkAuthState,
} from '../../store/deepLinkAuthState';
import { setupDesktopDeepLinkListener } from '../desktopDeepLinkListener';
import { storeSession } from '../tauriCommands';

const waitForAuthSettled = (): Promise<void> =>
  new Promise(resolve => {
    if (!getDeepLinkAuthState().isProcessing) {
      resolve();
      return;
    }
    const unsubscribe = subscribeDeepLinkAuthState(() => {
      if (!getDeepLinkAuthState().isProcessing) {
        unsubscribe();
        resolve();
      }
    });
  });

vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: () => ({ isBootstrapping: false, snapshot: { sessionToken: null } }),
  patchCoreStateSnapshot: vi.fn(),
}));

const windowControls = vi.hoisted(() => ({
  show: vi.fn().mockResolvedValue(undefined),
  unminimize: vi.fn().mockResolvedValue(undefined),
  setFocus: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => windowControls }));

describe('desktopDeepLinkListener', () => {
  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(getCurrent).mockResolvedValue(null);
    vi.mocked(onOpenUrl).mockResolvedValue(() => {});
    windowControls.show.mockClear();
    windowControls.unminimize.mockClear();
    windowControls.setFocus.mockClear();
    completeDeepLinkAuthProcessing();
  });

  it('turns Twitter OAuth error deep links into actionable UI and event diagnostics', async () => {
    const oauthErrorEvents: CustomEvent[] = [];
    window.addEventListener('oauth:error', event => {
      oauthErrorEvents.push(event as CustomEvent);
    });

    vi.mocked(getCurrent).mockResolvedValue([
      'openhuman://oauth/error?provider=twitter&error=invalid_request&callback_url=https%3A%2F%2Fexample.test%2Fcb%3Ftoken%3Dsecret',
    ]);

    await setupDesktopDeepLinkListener();

    expect(windowControls.show).toHaveBeenCalledTimes(1);
    expect(windowControls.unminimize).toHaveBeenCalledTimes(1);
    expect(windowControls.setFocus).toHaveBeenCalledTimes(1);
    expect(getDeepLinkAuthState()).toEqual({
      isProcessing: false,
      errorMessage:
        'Twitter/X sign-in failed before OpenHuman received authorization. Check the Twitter Developer Portal app settings: OAuth 2.0 must be enabled, callback URL must match the backend redirect URL exactly, and the client ID, client secret, and requested scopes must match the OpenHuman backend configuration.',
      requiresAppDataReset: false,
    });
    expect(oauthErrorEvents).toHaveLength(1);
    expect(oauthErrorEvents[0].detail).toEqual({
      provider: 'twitter',
      errorCode: 'invalid_request',
      message:
        'Twitter/X sign-in failed before OpenHuman received authorization. Check the Twitter Developer Portal app settings: OAuth 2.0 must be enabled, callback URL must match the backend redirect URL exactly, and the client ID, client secret, and requested scopes must match the OpenHuman backend configuration.',
    });
    expect(console.warn).toHaveBeenCalledWith(
      '[DeepLink][oauth:error] OAuth provider returned an error',
      expect.objectContaining({
        provider: 'twitter',
        errorCode: 'invalid_request',
        message: expect.stringContaining('Twitter Developer Portal app settings'),
      })
    );
    expect(JSON.stringify(vi.mocked(console.warn).mock.calls)).not.toContain('token%3Dsecret');
  });

  it('flags requiresAppDataReset when auth fails with a decryption error', async () => {
    vi.mocked(storeSession).mockRejectedValueOnce(
      new Error('Decryption failed — wrong key or tampered data')
    );

    vi.mocked(getCurrent).mockResolvedValue(['openhuman://auth?token=abc&key=auth']);

    await setupDesktopDeepLinkListener();

    await waitForAuthSettled();

    const state = getDeepLinkAuthState();
    expect(state.requiresAppDataReset).toBe(true);
    expect(state.errorMessage).toMatch(/Clear app data to start fresh/);
    expect(state.isProcessing).toBe(false);
  });

  it('keeps requiresAppDataReset false for non-decryption auth failures', async () => {
    vi.mocked(storeSession).mockRejectedValueOnce(new Error('network down'));

    vi.mocked(getCurrent).mockResolvedValue(['openhuman://auth?token=abc&key=auth']);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    const state = getDeepLinkAuthState();
    expect(state.requiresAppDataReset).toBe(false);
    expect(state.errorMessage).toBe('Sign-in failed. Please try again.');
  });

  it('sanitizes provider and error code values from OAuth error deep links', async () => {
    const oauthErrorEvents: CustomEvent[] = [];
    window.addEventListener('oauth:error', event => {
      oauthErrorEvents.push(event as CustomEvent);
    });

    vi.mocked(getCurrent).mockResolvedValue([
      'openhuman://oauth/error?provider=twit%20ter&error=bad%20request',
    ]);

    await setupDesktopDeepLinkListener();

    expect(oauthErrorEvents[0].detail).toEqual({
      provider: 'twit_ter',
      errorCode: 'bad_request',
      message:
        'OAuth sign-in failed before OpenHuman received authorization. Check the provider app settings and try again.',
    });
  });
});
