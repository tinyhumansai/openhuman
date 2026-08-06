import { isTauri } from '@tauri-apps/api/core';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { confirmWaitlistDownload } from '../../services/api/waitlistApi';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../../services/coreRpcClient';
import {
  completeDeepLinkAuthProcessing,
  getDeepLinkAuthState,
  subscribeDeepLinkAuthState,
} from '../../store/deepLinkAuthState';
import { getStoredCoreMode } from '../configPersistence';
import {
  authStoreFailureUserMessage,
  classifyAuthStoreFailure,
  handleDeepLinkUrls,
  registerAuthDeepLinkState,
  setupDesktopDeepLinkListener,
} from '../desktopDeepLinkListener';
import { BILLING_DASHBOARD_URL } from '../links';
import { openUrl } from '../openUrl';
import { storeSession } from '../tauriCommands';

vi.mock('../configPersistence', () => ({ getStoredCoreMode: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
}));
vi.mock('../openUrl', () => ({ openUrl: vi.fn() }));
vi.mock('../../services/api/waitlistApi', () => ({ confirmWaitlistDownload: vi.fn() }));

// Build an `openhuman://auth` deep link bound to a freshly registered state
// nonce, mirroring how the real OAuth button registers the loopback/deep-link
// state before the callback returns (finding C3 CSRF guard).
const authDeepLinkWithState = (query: string): string => {
  const state = registerAuthDeepLinkState();
  return `openhuman://auth?${query}&state=${state}`;
};

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

const waitForOAuthAuthReadiness = vi.hoisted(() =>
  vi.fn().mockResolvedValue({ ready: true as const })
);

vi.mock('../oauthAppVersionGate', async importOriginal => {
  const actual = await importOriginal<typeof import('../oauthAppVersionGate')>();
  return {
    ...actual,
    waitForOAuthAuthReadiness,
    oauthAuthReadinessUserMessage: (reason: string) => `blocked:${reason}`,
  };
});

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
    waitForOAuthAuthReadiness.mockReset();
    waitForOAuthAuthReadiness.mockResolvedValue({ ready: true });
    vi.mocked(storeSession).mockReset();
    vi.mocked(storeSession).mockResolvedValue(undefined);
    vi.mocked(getStoredCoreMode).mockReturnValue(null);
    vi.mocked(clearCoreRpcUrlCache).mockClear();
    vi.mocked(clearCoreRpcTokenCache).mockClear();
    vi.mocked(openUrl).mockReset();
    vi.mocked(openUrl).mockResolvedValue(undefined);
    vi.mocked(confirmWaitlistDownload).mockReset();
    vi.mocked(confirmWaitlistDownload).mockResolvedValue(undefined);
    windowControls.show.mockClear();
    windowControls.unminimize.mockClear();
    windowControls.setFocus.mockClear();
    completeDeepLinkAuthProcessing();
  });

  it('returns successful payment deep links to the billing dashboard', async () => {
    await handleDeepLinkUrls(['openhuman://payment/success?session_id=checkout-session']);

    expect(openUrl).toHaveBeenCalledWith(BILLING_DASHBOARD_URL);
    expect(BILLING_DASHBOARD_URL).toBe('https://tinyhumans.ai/dashboard');
  });

  it('returns cancelled payment deep links to the billing dashboard', async () => {
    await handleDeepLinkUrls(['openhuman://payment/cancel']);

    expect(openUrl).toHaveBeenCalledWith(BILLING_DASHBOARD_URL);
  });

  it('confirms the download and focuses the window for a waitlist deep link', async () => {
    await handleDeepLinkUrls(['openhuman://waitlist?token=dl-token-123']);

    expect(confirmWaitlistDownload).toHaveBeenCalledWith('dl-token-123');
    expect(windowControls.setFocus).toHaveBeenCalled();
  });

  it('still opens the app when the confirmation fails', async () => {
    // Startup must survive an offline launch: the reward is idempotent and the
    // next open retries it, but the window has to appear either way.
    vi.mocked(confirmWaitlistDownload).mockRejectedValue({ success: false, error: 'offline' });

    await expect(
      handleDeepLinkUrls(['openhuman://waitlist?token=dl-token-123'])
    ).resolves.toBeUndefined();
    expect(windowControls.setFocus).toHaveBeenCalled();
  });

  it('does not call the backend when the waitlist link carries no token', async () => {
    await handleDeepLinkUrls(['openhuman://waitlist']);

    expect(confirmWaitlistDownload).not.toHaveBeenCalled();
    expect(windowControls.setFocus).toHaveBeenCalled();
  });

  it('keeps the download token out of the logs on failure', async () => {
    // The token in the message is the point: a rejection on this path can carry
    // the credential, and `sanitizeError` would have preserved it.
    vi.mocked(confirmWaitlistDownload).mockRejectedValue(new Error('super-secret-token'));

    await handleDeepLinkUrls(['openhuman://waitlist?token=super-secret-token']);

    // Reads the suite-wide console spy from `src/test/setup.ts` rather than
    // installing its own. A local spy would have to be restored, and restoring
    // it un-spies `console.warn` for every test that runs after this one —
    // `restoreMocks` is off, so nothing puts it back.
    expect(console.warn).toHaveBeenCalledWith('[DeepLink][waitlist] Could not confirm download');
    expect(JSON.stringify(vi.mocked(console.warn).mock.calls)).not.toContain('super-secret-token');
  });

  it('waits for the core before confirming, and skips the call when it never comes up', async () => {
    // Cold open from a download link runs before BootCheckGate starts the core,
    // and the backend base resolves through it — confirming early would post to
    // nothing and be swallowed as an ordinary failure.
    waitForOAuthAuthReadiness.mockResolvedValue({ ready: false, reason: 'core_unreachable' });

    await handleDeepLinkUrls(['openhuman://waitlist?token=dl-token-123']);

    expect(confirmWaitlistDownload).not.toHaveBeenCalled();
    expect(windowControls.setFocus).toHaveBeenCalled();
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
      // Literal copy, not a key: only the localized failures carry one.
      errorMessageKey: null,
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

    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    await setupDesktopDeepLinkListener();

    await waitForAuthSettled();

    const state = getDeepLinkAuthState();
    expect(state.requiresAppDataReset).toBe(true);
    expect(state.errorMessage).toMatch(/Clear app data to start fresh/);
    expect(state.isProcessing).toBe(false);
  });

  it('surfaces readiness failures instead of a generic sign-in error', async () => {
    waitForOAuthAuthReadiness.mockResolvedValueOnce({ ready: false, reason: 'core_mode_unset' });

    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    await setupDesktopDeepLinkListener();

    const state = getDeepLinkAuthState();
    expect(state.errorMessage).toBe('blocked:core_mode_unset');
    expect(state.isProcessing).toBe(false);
    expect(storeSession).not.toHaveBeenCalled();
  });

  it('rejects an auth deep link with no state nonce (CSRF guard, finding C3)', async () => {
    // A hostile page can fire `openhuman://auth?token=<attacker_jwt>&key=auth`
    // with no state — it must never apply a session token.
    vi.mocked(getCurrent).mockResolvedValue(['openhuman://auth?token=attacker&key=auth']);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    expect(storeSession).not.toHaveBeenCalled();
    const state = getDeepLinkAuthState();
    expect(state.isProcessing).toBe(false);
    expect(state.errorMessage).toBe('Sign-in could not be verified. Please start sign-in again.');
  });

  it('accepts a same-origin web callback without a state nonce when requireStateNonce=false', async () => {
    // The web callback route (WebCallbackPage) is same-origin and not reachable
    // via the OS `openhuman://` scheme, so it opts out of the C3 nonce guard.
    await import('../desktopDeepLinkListener').then(m =>
      m.handleDeepLinkUrls(['openhuman://auth?token=web-token&key=auth'], {
        requireStateNonce: false,
      })
    );
    await waitForAuthSettled();

    expect(storeSession).toHaveBeenCalledWith(
      'web-token',
      {},
      { allowPendingBackendValidation: true, timeoutMs: 25_000 }
    );
  });

  it('retries storeSession on timeout then succeeds on second attempt', async () => {
    vi.mocked(storeSession)
      .mockReset()
      .mockRejectedValueOnce(new Error('timed out'))
      .mockResolvedValueOnce(undefined);

    const state = registerAuthDeepLinkState();
    const url = `openhuman://auth?token=retry-token&key=auth&state=${state}`;

    vi.mocked(getCurrent).mockResolvedValue([url]);
    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    expect(storeSession).toHaveBeenCalledTimes(2);
    expect(getDeepLinkAuthState().errorMessage).toBeNull();
  });

  it('does NOT retry storeSession on non-timeout error', async () => {
    vi.mocked(storeSession).mockReset().mockRejectedValueOnce(new Error('network down'));

    const state = registerAuthDeepLinkState();
    const url = `openhuman://auth?token=no-retry-token&key=auth&state=${state}`;

    vi.mocked(getCurrent).mockResolvedValue([url]);
    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    // Non-timeout errors should not be retried — only one call expected.
    expect(storeSession).toHaveBeenCalledTimes(1);
    expect(getDeepLinkAuthState().errorMessage).not.toBeNull();
  });

  it('rejects an auth deep link whose state nonce does not match a pending one', async () => {
    registerAuthDeepLinkState('the-real-nonce');
    vi.mocked(getCurrent).mockResolvedValue([
      'openhuman://auth?token=attacker&key=auth&state=wrong-nonce',
    ]);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    expect(storeSession).not.toHaveBeenCalled();
    expect(getDeepLinkAuthState().errorMessage).toBe(
      'Sign-in could not be verified. Please start sign-in again.'
    );
  });

  it('consumes a state nonce one-shot so a replayed deep link is rejected', async () => {
    const state = registerAuthDeepLinkState();
    const url = `openhuman://auth?token=abc&key=auth&state=${state}`;

    vi.mocked(getCurrent).mockResolvedValue([url]);
    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();
    expect(storeSession).toHaveBeenCalledWith(
      'abc',
      {},
      { allowPendingBackendValidation: true, timeoutMs: 25_000 }
    );

    // Replay the exact same deep link — the nonce was consumed, so it fails.
    vi.mocked(storeSession).mockClear();
    await import('../desktopDeepLinkListener').then(m => m.handleDeepLinkUrls([url]));
    await waitForAuthSettled();
    expect(storeSession).not.toHaveBeenCalled();
  });

  it('keeps requiresAppDataReset false for non-decryption auth failures', async () => {
    vi.mocked(storeSession).mockRejectedValueOnce(new Error('network down'));

    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    const state = getDeepLinkAuthState();
    expect(state.requiresAppDataReset).toBe(false);
    expect(state.errorMessage).toContain('did not respond');
    expect(state.errorMessage).toContain('restart');
    expect(state.errorMessageKey).toBeNull();
  });

  // The core cannot read its own config.toml: permanent, host-side, and
  // identical for every config-dependent RPC. It previously fell through to the
  // generic "Please try again", which is advice that can never work. The copy is
  // localized, and this module cannot call useT(), so it hands the rendering
  // component an i18n key instead of a literal.
  it('surfaces an unreadable core config as a translatable key, not a retry prompt', async () => {
    vi.mocked(storeSession).mockRejectedValueOnce(
      new Error(
        'Failed to read config file: /home/openhuman/.openhuman/config.toml ' +
          '[config owner mismatch] (file uid=0 gid=0 mode=0600; process euid=10001 egid=10001): ' +
          'Permission denied (os error 13)'
      )
    );

    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    const state = getDeepLinkAuthState();
    expect(state.errorMessageKey).toBe('welcome.coreConfigUnreadable');
    expect(state.errorMessage).not.toBe('Sign-in failed. Please try again.');
    expect(state.requiresAppDataReset).toBe(false);
  });

  it('injection #1: store-time /auth/me failure bounces to signin — no session applied, no /home nav', async () => {
    // Root-cause hypothesis: `auth_store_session` validates the JWT against the
    // backend GET /auth/me BEFORE persisting (credentials/ops.rs). If that call
    // errors/times out, store_session returns Err → applySessionToken rethrows →
    // the session is NEVER persisted and the login event NEVER fires, so the user
    // stays on the signin page even though OAuth "succeeded".
    vi.mocked(storeSession).mockRejectedValueOnce(
      new Error('Session validation failed (GET /auth/me): 503 Service Unavailable')
    );

    // The `core-state:session-token-updated` event is the ONLY trigger that drives
    // CoreStateProvider → refresh → authenticated React state. If it never fires,
    // the app cannot leave the signin page.
    const sessionTokenUpdated = vi.fn();
    window.addEventListener('core-state:session-token-updated', sessionTokenUpdated);
    window.location.hash = '#/'; // reset any prior test's navigation

    try {
      vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);
      await setupDesktopDeepLinkListener();
      await waitForAuthSettled();

      // store WAS attempted (we reached the persistence call)...
      expect(storeSession).toHaveBeenCalledWith(
        'abc',
        {},
        { allowPendingBackendValidation: true, timeoutMs: 25_000 }
      );
      // ...but it FAILED, so the session-applied event was never dispatched...
      expect(sessionTokenUpdated).not.toHaveBeenCalled();
      // ...and we never navigated to /home (ProtectedRoute/PublicRoute keep signin).
      expect(window.location.hash).not.toBe('#/home');
      // Surfaced as the generic toast; processing cleared.
      const state = getDeepLinkAuthState();
      expect(state.errorMessage).toContain('did not respond');
      expect(state.isProcessing).toBe(false);
    } finally {
      window.removeEventListener('core-state:session-token-updated', sessionTokenUpdated);
    }
  });

  it('does not make the E2E deep-link helper wait for auth readiness', async () => {
    let resolveReadiness!: (_value: { ready: true }) => void;
    waitForOAuthAuthReadiness.mockReturnValueOnce(
      new Promise<{ ready: true }>(resolve => {
        resolveReadiness = resolve;
      })
    );

    await setupDesktopDeepLinkListener();

    const simulateDeepLink = (
      window as Window & { __simulateDeepLink?: (url: string) => Promise<void> }
    ).__simulateDeepLink;

    expect(simulateDeepLink).toBeTypeOf('function');
    await expect(
      simulateDeepLink!('openhuman://auth?token=abc&key=auth&state=e2e-state-nonce')
    ).resolves.toBeUndefined();
    expect(storeSession).not.toHaveBeenCalled();

    await new Promise(resolve => setTimeout(resolve, 0));
    expect(waitForOAuthAuthReadiness).toHaveBeenCalledTimes(1);

    resolveReadiness({ ready: true });
    await waitForAuthSettled();

    expect(storeSession).toHaveBeenCalledWith(
      'abc',
      {},
      { allowPendingBackendValidation: true, timeoutMs: 25_000 }
    );
    expect(getDeepLinkAuthState().isProcessing).toBe(false);
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

  it('busts RPC caches before storeSession in cloud mode', async () => {
    vi.mocked(getStoredCoreMode).mockReturnValue('cloud');
    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    expect(clearCoreRpcUrlCache).toHaveBeenCalledTimes(1);
    expect(clearCoreRpcTokenCache).toHaveBeenCalledTimes(1);
    expect(storeSession).toHaveBeenCalledWith(
      'abc',
      {},
      { allowPendingBackendValidation: true, timeoutMs: 25_000 }
    );
  });

  it('does NOT bust RPC caches before storeSession in local mode', async () => {
    vi.mocked(getStoredCoreMode).mockReturnValue('local');
    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    expect(clearCoreRpcUrlCache).not.toHaveBeenCalled();
    expect(clearCoreRpcTokenCache).not.toHaveBeenCalled();
    expect(storeSession).toHaveBeenCalledWith(
      'abc',
      {},
      { allowPendingBackendValidation: true, timeoutMs: 25_000 }
    );
  });

  it('dispatches suppress-reauth before storeSession and clears it after in cloud mode', async () => {
    vi.mocked(getStoredCoreMode).mockReturnValue('cloud');
    vi.mocked(getCurrent).mockResolvedValue([authDeepLinkWithState('token=abc&key=auth')]);

    const suppressEvents: Array<{ until: number }> = [];
    window.addEventListener('core-state:suppress-reauth', event => {
      suppressEvents.push((event as CustomEvent<{ until: number }>).detail);
    });

    await setupDesktopDeepLinkListener();
    await waitForAuthSettled();

    // First event: non-zero until (suppress on)
    expect(suppressEvents.length).toBeGreaterThanOrEqual(2);
    expect(suppressEvents[0].until).toBeGreaterThan(0);
    // Last event: until=0 (suppress cleared)
    expect(suppressEvents[suppressEvents.length - 1].until).toBe(0);
  });
});

describe('classifyAuthStoreFailure', () => {
  it.each([
    ['Session validation failed (GET /auth/me): operation timed out', 'auth_me_timeout'],
    ['error sending request: deadline has elapsed', 'auth_me_timeout'],
    ['GET /auth/me failed (401 Unauthorized): bad token', 'auth_me_unauthorized'],
    ['Session validation failed (GET /auth/me): 503 Service Unavailable', 'auth_me_gateway'],
    ['upstream returned 502 Bad Gateway', 'auth_me_gateway'],
    ['fetch failed: ECONNREFUSED', 'network'],
    ['Session validation failed (GET /auth/me): something odd', 'auth_me_other'],
    ['totally unrelated explosion', 'other'],
  ])('classifies %j as %s', (message, expected) => {
    expect(classifyAuthStoreFailure(message)).toBe(expected);
  });

  // Contract pin: the classifier matches substrings of the Rust-produced error
  // (credentials/ops.rs: `Session validation failed (GET /auth/me): {reason}`,
  // with {reason} from rest.rs `GET /auth/me failed ({status}): {text}` or a
  // reqwest transport error). If Rust rewords that prefix, these must fail CI
  // rather than letting an arm silently degrade to 'other'.
  it('pins the real Rust store_session failure strings to meaningful kinds', () => {
    const gateway =
      'Session validation failed (GET /auth/me): GET /auth/me failed (503): {"error":"unavailable"}';
    const timeout =
      'Session validation failed (GET /auth/me): error sending request for url (https://api.tinyhumans.ai/auth/me): operation timed out';
    const bare = 'Session validation failed (GET /auth/me): something unexpected';

    expect(classifyAuthStoreFailure(gateway)).toBe('auth_me_gateway');
    expect(classifyAuthStoreFailure(timeout)).toBe('auth_me_timeout');
    // The bare prefix is still recognized via the auth/me anchor — NOT 'other'.
    expect(classifyAuthStoreFailure(bare)).toBe('auth_me_other');
    expect(classifyAuthStoreFailure(bare)).not.toBe('other');
  });

  // A core that cannot read its own config.toml fails EVERY config-dependent
  // RPC the same way. Bucketing it as 'other' surfaced "Sign-in failed. Please
  // try again." for a fault no amount of retrying clears.
  it('classifies an unreadable core config as its own permanent kind', () => {
    const reported =
      'Failed to read config file: /home/openhuman/.openhuman/config.toml ' +
      '[config owner mismatch] (file uid=0 gid=0 mode=0600; process euid=10001 egid=10001): ' +
      'Permission denied (os error 13)';

    expect(classifyAuthStoreFailure(reported)).toBe('config_unreadable');
    expect(classifyAuthStoreFailure(reported)).not.toBe('other');
  });
});

describe('authStoreFailureUserMessage (issue #3025)', () => {
  const CLOUD_KINDS = [
    'auth_me_timeout',
    'auth_me_unauthorized',
    'auth_me_gateway',
    'network',
    'other',
  ];

  // Local / unset mode should mention the retry attempt so the user knows
  // the system tried before giving up (issue #5166).
  it.each(['local', null] as const)('stays generic for mode=%s', mode => {
    for (const kind of CLOUD_KINDS) {
      const msg = authStoreFailureUserMessage(kind, mode);
      expect(msg).toContain('retry');
      expect(msg).not.toContain('remote');
    }
  });

  it('points cloud-mode users at the remote runtime, not a dead-end retry', () => {
    for (const kind of CLOUD_KINDS) {
      const msg = authStoreFailureUserMessage(kind, 'cloud');
      expect(msg).not.toBe('Sign-in failed. Please try again.');
      expect(msg.toLowerCase()).toContain('remote');
    }
  });

  it('gives kind-specific cloud hints (401 token, gateway/timeout BACKEND_URL)', () => {
    expect(authStoreFailureUserMessage('auth_me_unauthorized', 'cloud')).toContain('RPC token');
    expect(authStoreFailureUserMessage('auth_me_gateway', 'cloud')).toContain('BACKEND_URL');
    expect(authStoreFailureUserMessage('auth_me_timeout', 'cloud')).toContain('BACKEND_URL');
    expect(authStoreFailureUserMessage('network', 'cloud')).toContain('online');
  });
});
