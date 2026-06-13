import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { setCloudProviderKey } from '../../../../services/api/aiSettingsApi';
import { callCoreRpc } from '../../../../services/coreRpcClient';
import { renderWithProviders } from '../../../../test/test-utils';
import { openUrl } from '../../../../utils/openUrl';
import { isTauri } from '../../../../utils/tauriCommands/common';
import ApiKeysStep from '../ApiKeysStep';

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('../../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('../../../../utils/tauriCommands/common', () => ({ isTauri: vi.fn(() => true) }));

vi.mock('../../../../services/api/aiSettingsApi', () => ({
  setCloudProviderKey: vi.fn().mockResolvedValue(undefined),
}));

describe('ApiKeysStep OpenAI OAuth', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(openUrl).mockResolvedValue(undefined);
    vi.mocked(setCloudProviderKey).mockResolvedValue(undefined);
  });

  it('shows connected badge when oauth status reports connected', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { connected: true } });

    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    expect(await screen.findByTestId('onboarding-openai-oauth-connected')).toBeInTheDocument();
    expect(screen.getByText('Connected with ChatGPT')).toBeInTheDocument();
  });

  it('starts oauth and accepts pasted callback URL', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } })
      .mockResolvedValueOnce({
        result: {
          authUrl: 'https://auth.openai.com/oauth/authorize?client_id=test',
          state: 'state-1',
          redirectUri: 'http://127.0.0.1:1455/auth/callback',
        },
      })
      .mockResolvedValueOnce({ result: { connected: true } });

    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('onboarding-openai-oauth-connect'));

    await waitFor(() => {
      expect(openUrl).toHaveBeenCalledWith(
        'https://auth.openai.com/oauth/authorize?client_id=test'
      );
    });

    const input = await screen.findByTestId('onboarding-openai-oauth-callback-input');
    fireEvent.change(input, {
      target: { value: 'http://127.0.0.1:1455/auth/callback?code=abc&state=state-1' },
    });
    fireEvent.click(screen.getByTestId('onboarding-openai-oauth-complete'));

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.inference_openai_oauth_complete',
          params: { callback_url: 'http://127.0.0.1:1455/auth/callback?code=abc&state=state-1' },
        })
      );
    });

    expect(await screen.findByTestId('onboarding-openai-oauth-connected')).toBeInTheDocument();
  });

  it('shows a desktop-only error without calling core outside Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    fireEvent.click(screen.getByTestId('onboarding-openai-oauth-connect'));

    expect(
      await screen.findByText('ChatGPT sign-in is only available in the desktop app.')
    ).toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('reports an oauth start failure when core omits authUrl', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } })
      .mockResolvedValueOnce({ result: { authUrl: '   ' } });

    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('onboarding-openai-oauth-connect'));

    expect(
      await screen.findByText('Could not start ChatGPT sign-in. Try again or use an API key.')
    ).toBeInTheDocument();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('requires a pasted callback before completing oauth', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } })
      .mockResolvedValueOnce({
        result: { authUrl: 'https://auth.openai.com/oauth/authorize?client_id=test' },
      });

    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('onboarding-openai-oauth-connect'));
    await screen.findByTestId('onboarding-openai-oauth-callback-input');
    fireEvent.click(screen.getByTestId('onboarding-openai-oauth-complete'));

    expect(
      await screen.findByText('Paste the redirect URL from your browser after signing in.')
    ).toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.inference_openai_oauth_complete' })
    );
  });

  it('reports an oauth completion failure and keeps the callback form visible', async () => {
    vi.mocked(callCoreRpc)
      .mockResolvedValueOnce({ result: { connected: false } })
      .mockResolvedValueOnce({
        result: { authUrl: 'https://auth.openai.com/oauth/authorize?client_id=test' },
      })
      .mockRejectedValueOnce(new Error('state mismatch'));

    renderWithProviders(<ApiKeysStep onNext={vi.fn()} onSkip={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('onboarding-openai-oauth-connect'));
    const input = await screen.findByTestId('onboarding-openai-oauth-callback-input');
    fireEvent.change(input, {
      target: { value: 'http://127.0.0.1:1455/auth/callback?code=abc&state=wrong' },
    });
    fireEvent.click(screen.getByTestId('onboarding-openai-oauth-complete'));

    expect(
      await screen.findByText(
        'ChatGPT sign-in did not complete. Check the redirect URL and try again.'
      )
    ).toBeInTheDocument();
    expect(screen.getByTestId('onboarding-openai-oauth-callback-input')).toBeInTheDocument();
  });

  it('continues without saving API keys when oauth is already connected', async () => {
    const onNext = vi.fn();
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { connected: true } });

    renderWithProviders(<ApiKeysStep onNext={onNext} onSkip={vi.fn()} />);

    await screen.findByTestId('onboarding-openai-oauth-connected');
    fireEvent.click(screen.getByTestId('onboarding-next-button'));

    await waitFor(() => {
      expect(onNext).toHaveBeenCalledTimes(1);
    });
    expect(setCloudProviderKey).not.toHaveBeenCalled();
  });
});
