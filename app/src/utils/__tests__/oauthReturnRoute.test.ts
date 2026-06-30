import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  clearOAuthReturnRoute,
  setOAuthReturnRoute,
  takeOAuthReturnRoute,
} from '../oauthReturnRoute';

const STORAGE_KEY = 'openhuman:oauth:return-route';

describe('oauthReturnRoute', () => {
  afterEach(() => {
    sessionStorage.clear();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('stores a route and returns it once, clearing it afterwards', () => {
    setOAuthReturnRoute('/rewards');
    const stored = JSON.parse(sessionStorage.getItem(STORAGE_KEY) as string);
    expect(stored.route).toBe('/rewards');

    expect(takeOAuthReturnRoute()).toBe('/rewards');
    // Cleared after read → falls back to the default on the next call.
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('defaults to /connections when nothing is stored', () => {
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('ignores a stored value that is not an in-app path', () => {
    sessionStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ route: 'https://evil.example.com', ts: Date.now() })
    );
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('ignores corrupt (non-JSON) stored values', () => {
    sessionStorage.setItem(STORAGE_KEY, 'not-json');
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('ignores a stale route older than the freshness window', () => {
    sessionStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ route: '/rewards', ts: Date.now() - 6 * 60 * 1000 })
    );
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('clearOAuthReturnRoute forgets a stored route', () => {
    setOAuthReturnRoute('/rewards');
    clearOAuthReturnRoute();
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('falls back to the default when sessionStorage write throws', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('storage unavailable');
    });
    expect(() => setOAuthReturnRoute('/rewards')).not.toThrow();
  });

  it('falls back to the default when sessionStorage read throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage unavailable');
    });
    expect(takeOAuthReturnRoute()).toBe('/connections');
  });

  it('clearOAuthReturnRoute swallows storage errors', () => {
    vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
      throw new Error('storage unavailable');
    });
    expect(() => clearOAuthReturnRoute()).not.toThrow();
  });
});
