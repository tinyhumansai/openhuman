import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { getBackendUrl } from '../../../services/backendUrl';
import { getDeepLinkAuthState } from '../../../store/deepLinkAuthState';
import { openUrl } from '../../../utils/openUrl';
import { isTauri } from '../../../utils/tauriCommands';
import OAuthProviderButton from '../OAuthProviderButton';

vi.mock('../../../services/backendUrl', () => ({ getBackendUrl: vi.fn() }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

vi.mock('../../../utils/tauriCommands', () => ({ isTauri: vi.fn() }));

vi.mock('../../../store/deepLinkAuthState', () => ({ getDeepLinkAuthState: vi.fn() }));

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

describe('OAuthProviderButton', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(getBackendUrl).mockResolvedValue('https://backend.test');
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
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(getBackendUrl).toHaveBeenCalledTimes(1);
    expect(openUrl).toHaveBeenCalledWith(
      expect.stringMatching(/^https:\/\/backend\.test\/auth\/google\/login(\?.*)?$/)
    );
    expect(screen.getByRole('button', { name: /Connecting/ })).toBeDisabled();
  });

  it('resets isLoading when the window regains focus', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Google' }));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
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
      await Promise.resolve();
      await Promise.resolve();
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
      await Promise.resolve();
      await Promise.resolve();
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
      await Promise.resolve();
      await Promise.resolve();
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
    expect(getBackendUrl).not.toHaveBeenCalled();
    expect(openUrl).not.toHaveBeenCalled();
    expect(screen.queryByText('Connecting...')).not.toBeInTheDocument();
  });

  it('ignores rapid double-clicks while a request is in flight', async () => {
    render(<OAuthProviderButton provider={stubProvider} />);

    const button = screen.getByRole('button', { name: 'Google' });
    fireEvent.click(button);
    fireEvent.click(button);

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(getBackendUrl).toHaveBeenCalledTimes(1);
    expect(openUrl).toHaveBeenCalledTimes(1);
  });

  it('shows actionable Twitter diagnostics when OAuth startup fails', async () => {
    vi.mocked(openUrl).mockRejectedValue(
      new Error('failed to open openhuman://oauth/error?provider=twitter&token=secret')
    );

    render(<OAuthProviderButton provider={twitterProvider} />);

    fireEvent.click(screen.getByRole('button', { name: 'Twitter' }));

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      'Twitter/X sign-in could not start. Check that the Twitter OAuth app callback URL, client ID/secret, and requested scopes match the OpenHuman backend, then try again.'
    );
    expect(screen.getByRole('button', { name: 'Twitter' })).toBeEnabled();
    expect(console.error).toHaveBeenCalledWith(
      '[oauth-button][twitter] OAuth startup failed',
      expect.objectContaining({
        provider: 'twitter',
        providerName: 'Twitter',
        guidance: expect.stringContaining('Twitter/X sign-in could not start'),
        reason: expect.not.stringContaining('token=secret'),
      })
    );
  });
});
