import { describe, expect, it } from 'vitest';

import {
  isMetaOAuthToolkit,
  isOAuthRateLimitedError,
  metaOAuthRateLimitMessage,
} from './oauthHandoff';

describe('oauthHandoff', () => {
  it('detects Meta OAuth toolkits', () => {
    expect(isMetaOAuthToolkit('instagram')).toBe(true);
    expect(isMetaOAuthToolkit('Facebook')).toBe(true);
    expect(isMetaOAuthToolkit('gmail')).toBe(false);
  });

  it('detects OAuth rate-limit errors', () => {
    expect(isOAuthRateLimitedError(new Error('HTTP 429 Too Many Requests'))).toBe(true);
    expect(isOAuthRateLimitedError({ message: 'HTTP 429 Too Many Requests' })).toBe(true);
    expect(isOAuthRateLimitedError(new Error('rate_limit exceeded'))).toBe(true);
    expect(isOAuthRateLimitedError(new Error('401 Unauthorized'))).toBe(false);
  });

  it('builds Instagram-specific Meta rate-limit guidance', () => {
    const msg = metaOAuthRateLimitMessage('Instagram');
    expect(msg).toContain('429');
    expect(msg.toLowerCase()).toContain('business');
  });

  it('builds Facebook-specific Meta rate-limit guidance without Instagram account copy', () => {
    const msg = metaOAuthRateLimitMessage('Facebook');
    expect(msg).toContain('429');
    expect(msg).toContain('Facebook');
    expect(msg).toContain('Business Manager');
    expect(msg).not.toContain('Instagram Business or Creator');
  });
});
