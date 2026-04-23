import { afterEach, describe, expect, it, vi } from 'vitest';

import { sendEmailMagicLink } from '../authApi';

describe('sendEmailMagicLink', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('posts the email and redirect URI to the backend', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValue(new Response('{}', { status: 200 }) as Response);

    await sendEmailMagicLink('user@example.com', 'openhuman://');

    expect(fetchSpy).toHaveBeenCalledWith('http://localhost:5005/auth/email/send-link', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: 'user@example.com', frontendRedirectUri: 'openhuman://' }),
      signal: expect.any(AbortSignal),
    });
  });

  it('times out stalled requests and surfaces a retry message', async () => {
    vi.useFakeTimers();

    vi.spyOn(globalThis, 'fetch').mockImplementation((_input, init) => {
      const signal = init?.signal as AbortSignal | undefined;
      return new Promise((_resolve, reject) => {
        signal?.addEventListener('abort', () => {
          reject(new DOMException('The operation was aborted.', 'AbortError'));
        });
      });
    });

    const request = sendEmailMagicLink('user@example.com', 'openhuman://', 100);
    const rejection = expect(request).rejects.toThrow('Request timed out. Please try again.');

    await vi.advanceTimersByTimeAsync(100);

    await rejection;
  });
});
