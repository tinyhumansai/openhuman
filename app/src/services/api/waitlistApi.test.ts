import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockPost = vi.fn();
// Hoisted: `vi.mock` is lifted above this file's declarations, so the factory
// needs a binding that already exists when it runs.
const { mockLog } = vi.hoisted(() => ({ mockLog: vi.fn() }));

vi.mock('debug', () => ({ default: () => mockLog }));
vi.mock('../apiClient', () => ({ apiClient: { post: (...args: unknown[]) => mockPost(...args) } }));

describe('confirmWaitlistDownload', () => {
  beforeEach(() => {
    mockPost.mockReset();
    mockLog.mockReset();
  });

  it('posts the token to the confirm endpoint', async () => {
    mockPost.mockResolvedValueOnce({ success: true, data: {} });

    const { confirmWaitlistDownload } = await import('./waitlistApi');
    await confirmWaitlistDownload('tok_abc123');

    const [endpoint, body] = mockPost.mock.calls[0];
    expect(endpoint).toBe('/waitlist/tasks/download/confirm');
    expect(body).toEqual({ token: 'tok_abc123' });
  });

  it('sends no session bearer — the download token is the credential', async () => {
    mockPost.mockResolvedValueOnce({ success: true, data: {} });

    const { confirmWaitlistDownload } = await import('./waitlistApi');
    await confirmWaitlistDownload('tok_abc123');

    const options = mockPost.mock.calls[0][2] as { requireAuth?: boolean; timeout?: number };
    expect(options.requireAuth).toBe(false);
  });

  it('bounds the request at ten seconds so it cannot hold up app startup', async () => {
    mockPost.mockResolvedValueOnce({ success: true, data: {} });

    const { confirmWaitlistDownload } = await import('./waitlistApi');
    await confirmWaitlistDownload('tok_abc123');

    const options = mockPost.mock.calls[0][2] as { timeout?: number };
    expect(options.timeout).toBe(10_000);
  });

  it('logs the token length and never the token', async () => {
    mockPost.mockResolvedValueOnce({ success: true, data: {} });

    const { confirmWaitlistDownload } = await import('./waitlistApi');
    await confirmWaitlistDownload('tok_abc123');

    expect(mockLog).toHaveBeenCalledWith(expect.stringContaining('tokenLength'), 10);
    expect(JSON.stringify(mockLog.mock.calls)).not.toContain('tok_abc123');
  });

  it('propagates failures so the caller decides how to degrade', async () => {
    mockPost.mockRejectedValueOnce({ success: false, error: 'Waitlist entry not found' });

    const { confirmWaitlistDownload } = await import('./waitlistApi');
    await expect(confirmWaitlistDownload('tok_missing')).rejects.toEqual({
      success: false,
      error: 'Waitlist entry not found',
    });
  });
});
