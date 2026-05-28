import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { checkBackendHealthy } from '../../../services/backendHealth';
import {
  beginDeepLinkAuthProcessing,
  completeDeepLinkAuthProcessing,
  getDeepLinkAuthState,
} from '../../../store/deepLinkAuthState';
import { handleDeepLinkUrls } from '../../../utils/desktopDeepLinkListener';
import { startLoopbackOauthListener } from '../../../utils/loopbackOauthListener';
import { prepareOAuthLoginLaunch } from '../../../utils/oauthAppVersionGate';
import { openUrl } from '../../../utils/openUrl';
import { isTauri } from '../../../utils/tauriCommands';
import OAuthProviderButton from '../OAuthProviderButton';

vi.mock('../../../services/backendHealth', () => ({ checkBackendHealthy: vi.fn() }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

vi.mock('../../../utils/oauthAppVersionGate', () => ({
  prepareOAuthLoginLaunch: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../../utils/tauriCommands', () => ({ isTauri: vi.fn() }));

vi.mock('../../../utils/loopbackOauthListener', () => ({ startLoopbackOauthListener: vi.fn() }));

vi.mock('../../../utils/desktopDeepLinkListener', () => ({ handleDeepLinkUrls: vi.fn() }));

vi.mock('../../../store/deepLinkAuthState', () => ({
  beginDeepLinkAuthProcessing: vi.fn(),
  completeDeepLinkAuthProcessing: vi.fn(),
  getDeepLinkAuthState: vi.fn(),
}));

const stubProvider = {
  id: 'google' as const,
  name: 'Google',
  icon: ({ className }: { className?: string }) => (
    <span aria-hidden="true" className={className} />
  ),
  color: '',
  hoverColor: '',
  textColor: '',
  showOnWelcome: true,
};

const twitterProvider = { ...stubProvider, id: 'twitter' as const, name: 'Twitter' };

const healthyResult = {
  healthy: true as const,
  status: 200,
  latencyMs: 12,
  backendUrl: 'https://backend.test',
};

describe('OAuthProviderButton', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(checkBackendHealthy).mockResolvedValue(healthyResult);
    vi.mocked(openUrl).mockResolvedValue(undefined);
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(getDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it('opens the backend OAuth URL on click and shows Connecting...', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    const button = screen.getByRole('button', { name: 'Google' });
    fireEvent.click(button);

    // Drain the microtasks queued by the async click handler so openUrl resolves.
    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(beginDeepLinkAuthProcessing).toHaveBeenCalledTimes(1);
    expect(completeDeepLinkAuthProcessing).toHaveBeenCalledTimes(1);
    expect(prepareOAuthLoginLaunch).toHaveBeenCalledTimes(1);
    expect(checkBackendHealthy).toHaveBeenCalledTimes(1);
    expect(openUrl).toHaveBeenCalledWith(
      expect.stringMatching(/^https:\/\/backend\.test\/auth\/google\/login(\?.*)?$/)
    );
    expect(screen.getByRole('button', { name: /Connecting/ })).toBeDisabled();
  });

  it('resets isLoading when the window regains focus', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(screen.getByText('Connecting...')).toBeInTheDocument();

    await act(async () => {
      window.dispatchEvent(new FocusEvent('focus'));
    });

    expect(screen.queryByText('Connecting...')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Google' })).toBeEnabled();
  });

  it('does NOT reset isLoading on focus when a deep-link auth round-trip is processing', async () => {
    vi.mocked(getDeepLinkAuthState).mockReturnValue({
      isProcessing: true,
      errorMessage: null,
      requiresAppDataReset: false,
    });

    render(<OAuthProviderButton provider={stubProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(screen.getByText('Connecting...')).toBeInTheDocument();

    await act(async () => {
      window.dispatchEvent(new FocusEvent('focus'));
    });

    expect(screen.getByText('Connecting...')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Connecting/ })).toBeDisabled();
  });

  it('resets isLoading on visibilitychange to visible', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(screen.getByText('Connecting...')).toBeInTheDocument();

    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => 'visible',
    });
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });

    expect(screen.queryByText('Connecting...')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Google' })).toBeEnabled();
  });

  it('resets isLoading after the 90s safety timeout', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(screen.getByText('Connecting...')).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(90_000);
    });

    expect(screen.queryByText('Connecting...')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Google' })).toBeEnabled();
  });

  it('honors onClickOverride and skips the OAuth flow', () => {
    const override = vi.fn();

    render(<OAuthProviderButton provider={stubProvider} onClickOverride={override} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));

    expect(override).toHaveBeenCalledTimes(1);
    expect(checkBackendHealthy).not.toHaveBeenCalled();
    expect(openUrl).not.toHaveBeenCalled();
    expect(screen.queryByText('Connecting...')).not.toBeInTheDocument();
  });

  it('ignores rapid double-clicks while a request is in flight', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    const button = screen.getByRole('button', { name: 'Google' });
    fireEvent.click(button);
    fireEvent.click(button);

    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(checkBackendHealthy).toHaveBeenCalledTimes(1);
    expect(openUrl).toHaveBeenCalledTimes(1);
  });

  it('shows actionable Twitter diagnostics when OAuth startup fails', async () => {
    vi.mocked(openUrl).mockRejectedValue(
      new Error('failed to open openhuman://oauth/error?provider=twitter&token=secret')
    );

    render(<OAuthProviderButton provider={twitterProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Twitter' }));

    await act(async () => {
      // Drain enough microtasks to cover: checkBackendHealthy → getBackendUrl
      // → openUrl, plus any internal `.then` chains.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Twitter/X sign-in could not start. Check that the Twitter OAuth app callback URL, client ID/secret, and requested scopes match the OpenHuman backend, then try again.'
    );
    expect(screen.getByRole('button', { name: 'Twitter' })).toBeEnabled();
    expect(completeDeepLinkAuthProcessing).toHaveBeenCalledTimes(1);
  });

  it('surfaces safe readiness messages when the pre-launch readiness check fails', async () => {
    const readinessMessage =
      'OpenHuman could not reach its local runtime. Quit and reopen the app, then try signing in again.';
    vi.mocked(prepareOAuthLoginLaunch).mockRejectedValueOnce(new Error(readinessMessage));

    render(<OAuthProviderButton provider={stubProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));

    await act(async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(openUrl).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent(readinessMessage);
    expect(screen.getByRole('button', { name: 'Google' })).toBeEnabled();
    expect(completeDeepLinkAuthProcessing).toHaveBeenCalledTimes(1);
  });

  // --- Pre-flight + post-failure backend health probe (issue #1985) ---

  it.each([
    [
      'http-5xx',
      { healthy: false as const, reason: 'http-5xx' as const, status: 504, latencyMs: 1234 },
    ],
    ['timeout', { healthy: false as const, reason: 'timeout' as const, latencyMs: 4000 }],
    ['network', { healthy: false as const, reason: 'network' as const, latencyMs: 5 }],
    [
      'resolve-failure',
      { healthy: false as const, reason: 'resolve-failure' as const, latencyMs: 0 },
    ],
  ])(
    'pre-flight reason=%s blocks openUrl and shows the "temporarily unavailable" banner',
    async (_label, preflightResult) => {
      vi.mocked(checkBackendHealthy).mockResolvedValue(preflightResult);

      render(<OAuthProviderButton provider={stubProvider} />);
      fireEvent.click(screen.getByRole('button', { name: 'Google' }));

      await act(async () => {
        for (let i = 0; i < 6; i++) await Promise.resolve();
      });

      expect(openUrl).not.toHaveBeenCalled();
      expect(screen.getByRole('alert')).toHaveTextContent(
        /OpenHuman cloud sign-in is temporarily unavailable/i
      );
      expect(screen.getByRole('button', { name: 'Google' })).toBeEnabled();
    }
  );

  it('does NOT trigger a post-return probe when pre-flight blocked browser launch', async () => {
    vi.mocked(checkBackendHealthy).mockResolvedValueOnce({
      healthy: false,
      reason: 'http-5xx',
      status: 504,
      latencyMs: 1500,
    });

    render(<OAuthProviderButton provider={stubProvider} />);
    fireEvent.click(screen.getByRole('button', { name: 'Google' }));

    await act(async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    // Pre-flight ran exactly once for the click — the focus/visibility
    // handlers should NOT probe again because the browser was never opened.
    expect(checkBackendHealthy).toHaveBeenCalledTimes(1);

    // Even if the user happens to re-focus the window (e.g. they alt-tabbed),
    // we must not fire an additional probe — there's nothing for the user to
    // have returned from.
    //
    // The focus listener is attached only while isLoading=true, which the
    // pre-flight failure already cleared. So this is a regression guard:
    await act(async () => {
      window.dispatchEvent(new FocusEvent('focus'));
      for (let i = 0; i < 4; i++) await Promise.resolve();
    });
    expect(checkBackendHealthy).toHaveBeenCalledTimes(1);
  });

  it('after browser-return focus, surfaces the banner when the backend is unhealthy', async () => {
    // Happy pre-flight so the browser opens and the focus listener gets armed.
    vi.mocked(checkBackendHealthy)
      .mockResolvedValueOnce(healthyResult)
      .mockResolvedValueOnce({ healthy: false, reason: 'http-5xx', status: 504, latencyMs: 800 });

    render(<OAuthProviderButton provider={stubProvider} />);
    fireEvent.click(screen.getByRole('button', { name: 'Google' }));

    await act(async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });
    expect(openUrl).toHaveBeenCalledTimes(1);

    // User comes back from the browser without a deep-link round-trip.
    await act(async () => {
      window.dispatchEvent(new FocusEvent('focus'));
      // Drain microtasks for the background probe's then-chain.
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(checkBackendHealthy).toHaveBeenCalledTimes(2);
    expect(screen.getByRole('alert')).toHaveTextContent(
      /OpenHuman cloud sign-in is temporarily unavailable/i
    );
  });

  it('after browser-return focus, stays silent when the backend is healthy (user cancelled)', async () => {
    vi.mocked(checkBackendHealthy).mockResolvedValue(healthyResult);

    render(<OAuthProviderButton provider={stubProvider} />);
    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    await act(async () => {
      window.dispatchEvent(new FocusEvent('focus'));
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });

    expect(checkBackendHealthy).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('shows the unavailable banner if the 90s timeout fires while the backend is down', async () => {
    vi.mocked(checkBackendHealthy)
      .mockResolvedValueOnce(healthyResult)
      .mockResolvedValueOnce({ healthy: false, reason: 'timeout', latencyMs: 6000 });

    render(<OAuthProviderButton provider={stubProvider} />);
    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    });
    expect(openUrl).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(90_000);
      // After the safety timer fires we kick off probeBackendOnReturn().
      // Drain enough microtasks for that async probe to resolve and its
      // .then() to flush the alert into the DOM.
      for (let i = 0; i < 12; i++) await Promise.resolve();
    });

    expect(checkBackendHealthy).toHaveBeenCalledTimes(2);
    expect(screen.getByRole('alert')).toHaveTextContent(
      /OpenHuman cloud sign-in is temporarily unavailable/i
    );
  });

  it('appends redirectUri and routes loopback callback through handleDeepLinkUrls', async () => {
    let resolveCallback: ((url: string) => void) | null = null;
    vi.mocked(startLoopbackOauthListener).mockResolvedValue({
      redirectUri: 'http://127.0.0.1:53824/auth?state=abc',
      state: 'abc',
      awaitCallback: () =>
        new Promise<string>(resolve => {
          resolveCallback = resolve;
        }),
      cancel: vi.fn().mockResolvedValue(undefined),
    });

    render(<OAuthProviderButton provider={stubProvider} />);
    fireEvent.click(screen.getByRole('button', { name: 'Google' }));

    await act(async () => {
      for (let i = 0; i < 8; i++) await Promise.resolve();
    });

    expect(openUrl).toHaveBeenCalledWith(
      expect.stringContaining('redirectUri=http%3A%2F%2F127.0.0.1%3A53824%2Fauth%3Fstate%3Dabc')
    );

    // Simulate the shell emitting the callback for this listener.
    await act(async () => {
      resolveCallback!('http://127.0.0.1:53824/auth?token=jwt&state=abc');
      for (let i = 0; i < 4; i++) await Promise.resolve();
    });

    expect(handleDeepLinkUrls).toHaveBeenCalledWith(['openhuman://auth?token=jwt&state=abc']);
  });

  it('swallows loopback awaitCallback rejection without surfacing an error', async () => {
    vi.mocked(startLoopbackOauthListener).mockResolvedValue({
      redirectUri: 'http://127.0.0.1:53824/auth?state=x',
      state: 'x',
      awaitCallback: () => Promise.reject(new Error('loopback gone')),
      cancel: vi.fn().mockResolvedValue(undefined),
    });

    render(<OAuthProviderButton provider={stubProvider} />);
    fireEvent.click(screen.getByRole('button', { name: 'Google' }));

    await act(async () => {
      for (let i = 0; i < 8; i++) await Promise.resolve();
    });

    expect(openUrl).toHaveBeenCalledTimes(1);
    expect(handleDeepLinkUrls).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
