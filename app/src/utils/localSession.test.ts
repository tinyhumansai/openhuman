import { describe, expect, it } from 'vitest';

import {
  createLocalSessionToken,
  hasHostedAccount,
  isLocalSessionToken,
  LOCAL_SESSION_USER,
  LOCAL_SESSION_USER_ID,
} from './localSession';

function decodeBase64UrlJson<T>(value: string): T {
  const base64 = value.replace(/-/g, '+').replace(/_/g, '/');
  const padded = base64.padEnd(base64.length + ((4 - (base64.length % 4)) % 4), '=');
  return JSON.parse(globalThis.atob(padded)) as T;
}

describe('localSession', () => {
  it('creates a local JWT-shaped token with the local signature marker', () => {
    const token = createLocalSessionToken(1_700_000_000_000);
    const [headerPart, payloadPart, signaturePart] = token.split('.');

    expect(signaturePart).toBe('local');
    expect(isLocalSessionToken(token)).toBe(true);
    expect(decodeBase64UrlJson<Record<string, unknown>>(headerPart)).toEqual({
      alg: 'none',
      typ: 'JWT',
    });
    expect(decodeBase64UrlJson<Record<string, unknown>>(payloadPart)).toMatchObject({
      sub: LOCAL_SESSION_USER_ID,
      user_id: LOCAL_SESSION_USER_ID,
      iat: 1_700_000_000,
      exp: 1_731_536_000,
    });
  });

  it('requires exactly three token parts and a local marker', () => {
    expect(isLocalSessionToken('header.payload.local')).toBe(true);
    expect(isLocalSessionToken('header.payload.remote')).toBe(false);
    expect(isLocalSessionToken('header.payload.local.extra')).toBe(false);
    expect(isLocalSessionToken('not-a-jwt')).toBe(false);
    expect(isLocalSessionToken(null)).toBe(false);
  });

  it('exports the local user payload used by the welcome flow', () => {
    expect(LOCAL_SESSION_USER).toEqual({
      _id: LOCAL_SESSION_USER_ID,
      id: LOCAL_SESSION_USER_ID,
      name: 'Local User',
      email: 'local@openhuman.local',
    });
  });
});

describe('hasHostedAccount', () => {
  const local = createLocalSessionToken(1_700_000_000_000);

  it('is false when signed out', () => {
    expect(
      hasHostedAccount({ auth: { isAuthenticated: false, credential: null }, sessionToken: null })
    ).toBe(false);
  });

  it('follows the credential kind the core reports', () => {
    expect(
      hasHostedAccount({
        auth: { isAuthenticated: true, credential: 'session' },
        sessionToken: 'a.b.c',
      })
    ).toBe(true);
    expect(
      hasHostedAccount({
        auth: { isAuthenticated: true, credential: 'api-key' },
        sessionToken: null,
      })
    ).toBe(true);
    expect(
      hasHostedAccount({
        auth: { isAuthenticated: true, credential: 'local' },
        sessionToken: local,
      })
    ).toBe(false);
  });

  it('falls back to the token shape when an older core omits the credential', () => {
    expect(hasHostedAccount({ auth: { isAuthenticated: true }, sessionToken: local })).toBe(false);
    expect(hasHostedAccount({ auth: { isAuthenticated: true }, sessionToken: 'a.b.c' })).toBe(true);
  });
});
