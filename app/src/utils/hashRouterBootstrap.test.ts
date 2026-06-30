import { describe, expect, it } from 'vitest';

import { missingHashRedirectTarget } from './hashRouterBootstrap';

describe('missingHashRedirectTarget', () => {
  it('moves top-level auth callback query params into the hash route', () => {
    expect(missingHashRedirectTarget('/auth', '?token=jwt-token&key=auth')).toBe(
      '/#/auth?token=jwt-token&key=auth'
    );
  });

  it('normalizes a trailing slash on the top-level auth callback route', () => {
    expect(missingHashRedirectTarget('/auth/', '?token=jwt-token')).toBe('/#/auth?token=jwt-token');
  });

  it('keeps the existing default hash rewrite for non-callback routes', () => {
    expect(missingHashRedirectTarget('/settings', '?tab=voice')).toBe('/settings?tab=voice#/');
  });
});
