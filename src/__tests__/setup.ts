/**
 * Global test setup — runs before every test file.
 *
 * - Silences console output during tests (unless DEBUG_TESTS=1)
 * - Resets rate limiter module-level state between tests
 */
import { beforeEach, vi } from 'vitest';

// Silence console during tests to keep output clean
if (!process.env.DEBUG_TESTS) {
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'error').mockImplementation(() => {});
}

// Reset rate limiter per-request counter before each test.
// The module keeps mutable state (callHistory, lastCallTime, callsInCurrentRequest)
// that leaks between tests if not cleared.
beforeEach(async () => {
  try {
    const { resetRequestCallCount } = await import('../lib/mcp/rateLimiter');
    if (typeof resetRequestCallCount === 'function') {
      resetRequestCallCount();
    }
  } catch {
    // Module may be fully mocked in some test files — safe to skip
  }
});
