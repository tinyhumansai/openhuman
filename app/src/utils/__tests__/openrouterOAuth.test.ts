import { describe, expect, it, vi } from 'vitest';

import { connectOpenRouterViaOAuth } from '../openrouterOAuth';

describe('connectOpenRouterViaOAuth', () => {
  it('opens the OpenRouter auth URL and exchanges the callback code for an API key', async () => {
    const openExternalUrl = vi.fn().mockResolvedValue(undefined);
    const cancel = vi.fn().mockResolvedValue(undefined);
    const startLoopbackListener = vi
      .fn()
      .mockResolvedValue({
        redirectUri: 'http://127.0.0.1:53824/auth?state=expected-state',
        state: 'expected-state',
        awaitCallback: vi
          .fn()
          .mockResolvedValue('http://127.0.0.1:53824/auth?state=expected-state&code=abc123'),
        cancel,
      });
    const fetchImpl = vi
      .fn()
      .mockResolvedValue({ ok: true, json: async () => ({ key: 'sk-or-via-oauth' }) });

    const key = await connectOpenRouterViaOAuth({
      startLoopbackListener,
      openExternalUrl,
      fetchImpl: fetchImpl as unknown as typeof fetch,
    });

    expect(key).toBe('sk-or-via-oauth');
    expect(startLoopbackListener).toHaveBeenCalledWith({ port: 3000 });
    expect(openExternalUrl).toHaveBeenCalledTimes(1);
    const authUrl = new URL(openExternalUrl.mock.calls[0][0]);
    expect(authUrl.origin + authUrl.pathname).toBe('https://openrouter.ai/auth');
    // callback_url must use the port the listener actually bound to (carried in
    // redirectUri), not the requested OPENROUTER_LOOPBACK_PORT constant.
    expect(authUrl.searchParams.get('callback_url')).toBe(
      'http://localhost:53824/auth?state=expected-state'
    );
    expect(authUrl.searchParams.get('code_challenge_method')).toBe('S256');
    expect(authUrl.searchParams.get('code_challenge')).toBeTruthy();
    expect(fetchImpl).toHaveBeenCalledWith(
      'https://openrouter.ai/api/v1/auth/keys',
      expect.objectContaining({ method: 'POST', headers: { 'Content-Type': 'application/json' } })
    );
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it('uses the listener bound port for callback_url when the requested port was busy', async () => {
    // Port 3000 busy -> the Tauri listener falls back to an OS-assigned port and
    // returns it in redirectUri. callback_url must point at that bound port, or
    // OpenRouter redirects to a port nothing is listening on and OAuth fails.
    const openExternalUrl = vi.fn().mockResolvedValue(undefined);
    const startLoopbackListener = vi
      .fn()
      .mockResolvedValue({
        redirectUri: 'http://127.0.0.1:54321/auth?state=expected-state',
        state: 'expected-state',
        awaitCallback: vi
          .fn()
          .mockResolvedValue('http://127.0.0.1:54321/auth?state=expected-state&code=abc123'),
        cancel: vi.fn().mockResolvedValue(undefined),
      });
    const fetchImpl = vi
      .fn()
      .mockResolvedValue({ ok: true, json: async () => ({ key: 'sk-or-via-oauth' }) });

    await connectOpenRouterViaOAuth({
      startLoopbackListener,
      openExternalUrl,
      fetchImpl: fetchImpl as unknown as typeof fetch,
    });

    const authUrl = new URL(openExternalUrl.mock.calls[0][0]);
    expect(authUrl.searchParams.get('callback_url')).toBe(
      'http://localhost:54321/auth?state=expected-state'
    );
  });

  it('rejects when the loopback listener is unavailable', async () => {
    await expect(
      connectOpenRouterViaOAuth({ startLoopbackListener: vi.fn().mockResolvedValue(null) })
    ).rejects.toThrow('OpenRouter OAuth requires the desktop app');
  });

  it('rejects when the callback state does not match the request', async () => {
    const cancel = vi.fn().mockResolvedValue(undefined);

    await expect(
      connectOpenRouterViaOAuth({
        startLoopbackListener: vi
          .fn()
          .mockResolvedValue({
            redirectUri: 'http://127.0.0.1:53824/auth?state=expected-state',
            state: 'expected-state',
            awaitCallback: vi
              .fn()
              .mockResolvedValue('http://127.0.0.1:53824/auth?state=wrong-state&code=abc123'),
            cancel,
          }),
        openExternalUrl: vi.fn().mockResolvedValue(undefined),
        fetchImpl: vi.fn() as unknown as typeof fetch,
      })
    ).rejects.toThrow('OpenRouter OAuth callback state did not match the request.');

    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it('cancels the loopback listener when the OAuth flow is aborted', async () => {
    const cancel = vi.fn().mockResolvedValue(undefined);
    const controller = new AbortController();

    const promise = connectOpenRouterViaOAuth({
      signal: controller.signal,
      startLoopbackListener: vi
        .fn()
        .mockResolvedValue({
          redirectUri: 'http://127.0.0.1:3000/auth?state=expected-state',
          state: 'expected-state',
          awaitCallback: vi.fn().mockImplementation(() => new Promise(() => {})),
          cancel,
        }),
      openExternalUrl: vi.fn().mockResolvedValue(undefined),
      fetchImpl: vi.fn() as unknown as typeof fetch,
    });

    controller.abort();

    await expect(promise).rejects.toThrow('OpenRouter OAuth was cancelled.');
    expect(cancel).toHaveBeenCalledTimes(1);
  });
});
