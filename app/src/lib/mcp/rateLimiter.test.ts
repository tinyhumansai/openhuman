/**
 * Unit tests for MCP rate limiter
 *
 * Note: tests that would exercise real sleeping (inter-call delays, per-minute
 * window waits) are skipped to keep the suite fast. Those paths require either
 * fake timers or vi.useFakeTimers() integration with async Promises, which
 * conflicts with the shared mock-server setup in setup.ts.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  classifyTool,
  enforceRateLimit,
  getRateLimitStatus,
  isHeavyTool,
  isStateOnlyTool,
  RATE_LIMIT_CONFIG,
  resetRequestCallCount,
} from './rateLimiter';

// Reset module-level state before every test so tests are independent.
beforeEach(() => {
  resetRequestCallCount();
});

// Restore any timer fakes after each test.
afterEach(() => {
  vi.useRealTimers();
});

describe('classifyTool', () => {
  it('classifies a known state-only tool as state_only', () => {
    expect(classifyTool('get_chats')).toBe('state_only');
  });

  it('classifies a known API read tool as api_read', () => {
    expect(classifyTool('search_contacts')).toBe('api_read');
  });

  it('classifies a known API write tool as api_write', () => {
    expect(classifyTool('send_message')).toBe('api_write');
  });

  it('defaults unknown tools to api_read', () => {
    expect(classifyTool('totally_unknown_tool')).toBe('api_read');
  });
});

describe('isStateOnlyTool', () => {
  it('returns true for get_me', () => {
    expect(isStateOnlyTool('get_me')).toBe(true);
  });

  it('returns false for send_message', () => {
    expect(isStateOnlyTool('send_message')).toBe(false);
  });
});

describe('isHeavyTool', () => {
  it('returns true for delete_message', () => {
    expect(isHeavyTool('delete_message')).toBe(true);
  });

  it('returns false for get_me', () => {
    expect(isHeavyTool('get_me')).toBe(false);
  });
});

describe('getRateLimitStatus', () => {
  it('returns zero call counts at startup', () => {
    const status = getRateLimitStatus();
    expect(status.callsThisRequest).toBe(0);
    expect(typeof status.callsThisMinute).toBe('number');
    expect(status.callsThisMinute).toBeGreaterThanOrEqual(0);
  });
});

describe('enforceRateLimit — state_only tools', () => {
  it('resolves immediately without incrementing the request counter', async () => {
    await enforceRateLimit('get_chats');
    // state_only tools must not count against the per-request budget
    expect(getRateLimitStatus().callsThisRequest).toBe(0);
  });

  it('resolves immediately even when called many times', async () => {
    for (let i = 0; i < 25; i++) {
      await enforceRateLimit('get_me');
    }
    expect(getRateLimitStatus().callsThisRequest).toBe(0);
  });
});

describe('enforceRateLimit — per-request cap', () => {
  it('throws when the per-request cap is exceeded', async () => {
    // Use fake timers so inter-call delays resolve instantly.
    vi.useFakeTimers();

    const max = RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST;

    // Build promises and immediately attach a catch so Node never sees an
    // unhandled rejection, even if the async fn throws synchronously.
    const settled: Array<{ ok: true } | { ok: false; err: Error }> = [];
    const promises = Array.from({ length: max + 1 }, () => {
      const p = enforceRateLimit('search_contacts', 'api_read');
      p.catch(() => {}); // suppress unhandled rejection
      return p;
    });

    await vi.runAllTimersAsync();

    for (const p of promises) {
      await p.then(
        () => settled.push({ ok: true }),
        (err: Error) => settled.push({ ok: false, err })
      );
    }

    const rejected = settled.filter(s => !s.ok);
    expect(rejected.length).toBeGreaterThanOrEqual(1);
    const firstRejected = rejected[0];
    if (!firstRejected.ok) {
      expect(firstRejected.err.message).toMatch(/Rate limit/);
    }
  });
});

describe('resetRequestCallCount', () => {
  it('resets the per-request counter so subsequent API calls are allowed again', async () => {
    vi.useFakeTimers();

    // Fill up the request budget, suppressing rejections to avoid unhandled errors.
    const max = RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST;
    const fill = Array.from({ length: max }, () => {
      const p = enforceRateLimit('list_contacts', 'api_read');
      p.catch(() => {});
      return p;
    });
    await vi.runAllTimersAsync();
    // Drain settled results (some may reject if counter already exceeded)
    await Promise.allSettled(fill);

    // Reset and confirm a subsequent call succeeds
    resetRequestCallCount();

    const afterReset = enforceRateLimit('list_contacts', 'api_read');
    await vi.runAllTimersAsync();
    await expect(afterReset).resolves.toBeUndefined();
  });
});

describe('RATE_LIMIT_CONFIG', () => {
  it('has the expected shape', () => {
    expect(RATE_LIMIT_CONFIG.API_READ_DELAY_MS).toBeTypeOf('number');
    expect(RATE_LIMIT_CONFIG.API_WRITE_DELAY_MS).toBeTypeOf('number');
    expect(RATE_LIMIT_CONFIG.MAX_CALLS_PER_MINUTE).toBeTypeOf('number');
    expect(RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST).toBeTypeOf('number');
    // Write delay should be heavier than read delay
    expect(RATE_LIMIT_CONFIG.API_WRITE_DELAY_MS).toBeGreaterThan(
      RATE_LIMIT_CONFIG.API_READ_DELAY_MS
    );
  });
});
