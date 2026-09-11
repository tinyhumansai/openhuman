import { beforeEach, describe, expect, test, vi } from 'vitest';

import { DISCORD_INVITE_URL } from './links';

/**
 * These read the REAL `config.ts`, deliberately.
 *
 * Every other test that touches `SUPPORT_URL` mocks the module
 * (`ErrorFallbackScreen.test.tsx` and `analytics.test.ts` both `vi.mock`
 * `../utils/config`, which resolves to this same file), so none of them can
 * observe the shipped value — reverting the constant to the dead
 * `https://tinyhumans.ai/support` would leave the whole suite green. This file
 * is what makes that a failing test (#5870, #5953).
 *
 * The import is dynamic, after an explicit unmock and module reset, because a
 * sibling file's `vi.mock` of the same path otherwise reaches this one and the
 * assertions run against the stub instead of the real constant — which would
 * make them vacuous in exactly the way they exist to prevent.
 */
async function realConfig() {
  vi.resetModules();
  vi.doUnmock('./config');
  vi.doUnmock('../utils/config');
  return import('./config');
}

describe('SUPPORT_URL (real config)', () => {
  beforeEach(() => {
    vi.resetModules();
  });

  test('defaults to the community Discord, not the retired /support page', async () => {
    // `https://tinyhumans.ai/support` 404s — that is the bug #5870 reported.
    const { SUPPORT_URL } = await realConfig();
    expect(SUPPORT_URL).not.toContain('tinyhumans.ai/support');
    expect(SUPPORT_URL).toBe(DISCORD_INVITE_URL);
  });

  test('is a single source of truth shared with links.ts', async () => {
    // Not merely equal by coincidence: config.ts imports the constant, so the
    // vanity domain moves in one edit rather than two that can drift.
    const { SUPPORT_URL } = await realConfig();
    expect(SUPPORT_URL).toBe('https://discord.tinyhumans.ai');
  });

  test('does not claim to consume a ref by default', async () => {
    // With no VITE_SUPPORT_URL override the destination is a Discord invite,
    // which ignores the query. Callers key the `?ref=<sentryEventId>` append on
    // this flag, so a default build must not advertise it.
    const { SUPPORT_URL_ACCEPTS_REF } = await realConfig();
    expect(SUPPORT_URL_ACCEPTS_REF).toBe(false);
  });
});
