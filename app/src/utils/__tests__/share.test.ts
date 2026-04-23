import { beforeEach, describe, expect, it, vi } from 'vitest';

import { buildInviteUrl, buildShareUrl, copyToClipboard, tryNativeShare } from '../share';

vi.mock('../openUrl', () => ({ openUrl: vi.fn() }));

describe('buildInviteUrl', () => {
  it('appends the invite code to the share base URL', () => {
    expect(buildInviteUrl('ABC123')).toBe('https://openhuman.ai/i/ABC123');
  });

  it('trims whitespace', () => {
    expect(buildInviteUrl('  HELLO  ')).toBe('https://openhuman.ai/i/HELLO');
  });

  it('url-encodes special characters', () => {
    expect(buildInviteUrl('a/b c')).toBe('https://openhuman.ai/i/a%2Fb%20c');
  });

  it('falls back to the base URL for empty codes', () => {
    expect(buildInviteUrl('')).toBe('https://openhuman.ai');
    expect(buildInviteUrl('   ')).toBe('https://openhuman.ai');
  });
});

describe('buildShareUrl', () => {
  const text = 'Try OpenHuman';
  const url = 'https://openhuman.ai/i/XYZ';

  it('builds twitter intent', () => {
    const result = buildShareUrl('twitter', text, url);
    expect(result).toContain('https://twitter.com/intent/tweet');
    expect(result).toContain(encodeURIComponent(text));
    expect(result).toContain(encodeURIComponent(url));
  });

  it('builds telegram share url', () => {
    const result = buildShareUrl('telegram', text, url);
    expect(result).toContain('https://t.me/share/url');
    expect(result).toContain(encodeURIComponent(url));
  });

  it('builds whatsapp share url', () => {
    const result = buildShareUrl('whatsapp', text, url);
    expect(result).toContain('https://wa.me/?text=');
    expect(result).toContain(encodeURIComponent(`${text} ${url}`));
  });

  it('builds a mailto: email link', () => {
    const result = buildShareUrl('email', text, url);
    expect(result.startsWith('mailto:')).toBe(true);
    expect(result).toContain('subject=');
    expect(result).toContain('body=');
  });
});

describe('copyToClipboard', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('uses navigator.clipboard when available', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    await expect(copyToClipboard('hello')).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith('hello');
  });

  it('returns false when no copy mechanism works', async () => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: vi.fn().mockRejectedValue(new Error('blocked')) },
      configurable: true,
    });
    const originalExecCommand = document.execCommand;
    document.execCommand = vi.fn().mockImplementation(() => {
      throw new Error('nope');
    }) as typeof document.execCommand;
    try {
      await expect(copyToClipboard('hello')).resolves.toBe(false);
    } finally {
      document.execCommand = originalExecCommand;
    }
  });
});

describe('tryNativeShare', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('returns false when navigator.share is missing', async () => {
    const original = (navigator as Navigator & { share?: unknown }).share;
    Object.defineProperty(navigator, 'share', { value: undefined, configurable: true });
    try {
      await expect(tryNativeShare({ url: 'x' })).resolves.toBe(false);
    } finally {
      Object.defineProperty(navigator, 'share', { value: original, configurable: true });
    }
  });

  it('treats AbortError as a completed share', async () => {
    const err = new Error('cancelled');
    err.name = 'AbortError';
    const share = vi.fn().mockRejectedValue(err);
    Object.defineProperty(navigator, 'share', { value: share, configurable: true });
    await expect(tryNativeShare({ url: 'x' })).resolves.toBe(true);
    expect(share).toHaveBeenCalled();
  });
});
