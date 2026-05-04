import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { toolExecutionTimeoutMsFromEnv, withTimeout } from './withTimeout';

// Provide TOOL_TIMEOUT_SECS that the global setup mock omits.
vi.mock('./config', async importOriginal => {
  const actual = await importOriginal<typeof import('./config')>();
  return { ...actual, TOOL_TIMEOUT_SECS: 120 };
});

describe('withTimeout', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('resolves with the promise value when it settles before the timeout', async () => {
    const promise = Promise.resolve('ok');
    const result = await withTimeout(promise, 5000, 'fast-op');
    expect(result).toBe('ok');
  });

  it('rejects with a timeout error when the timeout elapses first', async () => {
    // withTimeout creates an internal timeoutPromise that rejects. When
    // Promise.race settles via that rejection, the internal promise itself
    // has no handler yet — Node surfaces it as an unhandled rejection warning.
    // We suppress it by attaching a catch to the overall race before advancing
    // the timer, then asserting on the stored error.
    let capturedError: unknown;
    const never = new Promise<never>(() => {});

    const racePromise = withTimeout(never, 3000, 'slow-op').catch((e: unknown) => {
      capturedError = e;
      // Swallow here — we assert manually below.
    });

    await vi.advanceTimersByTimeAsync(3001);
    await racePromise; // wait for the catch branch to complete

    expect(capturedError).toBeInstanceOf(Error);
    expect((capturedError as Error).message).toBe('slow-op timed out after 3s');
  });

  it('error message rounds timeout to seconds', async () => {
    let capturedError: unknown;
    const never = new Promise<never>(() => {});

    const racePromise = withTimeout(never, 2500, 'half-sec').catch((e: unknown) => {
      capturedError = e;
    });

    await vi.advanceTimersByTimeAsync(3000);
    await racePromise;

    expect(capturedError).toBeInstanceOf(Error);
    // Math.round(2500 / 1000) === 3
    expect((capturedError as Error).message).toBe('half-sec timed out after 3s');
  });

  it('passes through a rejection from the underlying promise', async () => {
    const failing = Promise.reject(new Error('upstream failure'));
    failing.catch(() => {}); // suppress premature unhandled warning
    await expect(withTimeout(failing, 5000, 'fail-op')).rejects.toThrow('upstream failure');
  });

  it('does not fire the timeout when the promise rejects quickly', async () => {
    const err = new Error('quick rejection');
    const failing = Promise.reject(err);
    failing.catch(() => {}); // suppress premature unhandled warning
    const racePromise = withTimeout(failing, 5000, 'quick-fail');

    await expect(racePromise).rejects.toThrow('quick rejection');

    // Advance well past the timeout — timer should already be cleared.
    await vi.advanceTimersByTimeAsync(10000);
  });

  it('bypasses the timeout when timeoutMs <= 0 and returns the raw promise', async () => {
    const promise = Promise.resolve(42);
    const result = await withTimeout(promise, 0, 'zero-timeout');
    expect(result).toBe(42);
  });

  it('bypasses the timeout for negative timeoutMs', async () => {
    const promise = Promise.resolve('neg');
    const result = await withTimeout(promise, -1, 'negative-timeout');
    expect(result).toBe('neg');
  });

  it('resolves with complex return types', async () => {
    const payload = { id: 1, data: [true, 'hello'] };
    const result = await withTimeout(Promise.resolve(payload), 1000, 'complex');
    expect(result).toEqual(payload);
  });

  it('clears the timer when the promise resolves (no dangling timer)', async () => {
    const clearSpy = vi.spyOn(globalThis, 'clearTimeout');
    await withTimeout(Promise.resolve('done'), 5000, 'cleanup-check');
    expect(clearSpy).toHaveBeenCalled();
  });
});

describe('toolExecutionTimeoutMsFromEnv', () => {
  it('returns a positive number of milliseconds', () => {
    const ms = toolExecutionTimeoutMsFromEnv();
    expect(ms).toBeGreaterThan(0);
    expect(Number.isFinite(ms)).toBe(true);
  });

  it('returns 120 000 ms matching the mocked TOOL_TIMEOUT_SECS of 120', () => {
    const ms = toolExecutionTimeoutMsFromEnv();
    // TOOL_TIMEOUT_SECS is mocked to 120 by this file's vi.mock above.
    expect(ms).toBe(120_000);
  });
});
