import { beforeEach, describe, expect, test, vi } from 'vitest';

// Hoisted mocks so tests can swap return values per case.
const hoisted = vi.hoisted(() => ({
  getClient: vi.fn(),
  captureException: vi.fn(),
  flush: vi.fn(() => Promise.resolve(true)),
}));

vi.mock('@sentry/react', () => ({
  getClient: hoisted.getClient,
  captureException: hoisted.captureException,
  flush: hoisted.flush,
}));

describe('triggerSentryTestEvent', () => {
  beforeEach(() => {
    hoisted.getClient.mockReset();
    hoisted.captureException.mockReset();
    hoisted.flush.mockReset();
    hoisted.flush.mockReturnValue(Promise.resolve(true));
  });

  test('returns undefined when Sentry client is not initialized', async () => {
    hoisted.getClient.mockReturnValue(undefined);
    const { triggerSentryTestEvent } = await import('../analytics');

    const result = await triggerSentryTestEvent();

    expect(result).toBeUndefined();
    expect(hoisted.captureException).not.toHaveBeenCalled();
    expect(hoisted.flush).not.toHaveBeenCalled();
  });

  test('captures a tagged staging-test exception and flushes', async () => {
    hoisted.getClient.mockReturnValue({});
    hoisted.captureException.mockReturnValue('event-id-abc');
    const { triggerSentryTestEvent } = await import('../analytics');

    const result = await triggerSentryTestEvent();

    expect(result).toBe('event-id-abc');
    expect(hoisted.captureException).toHaveBeenCalledTimes(1);

    const [thrown, ctx] = hoisted.captureException.mock.calls[0];
    expect(thrown).toBeInstanceOf(Error);
    expect((thrown as Error).name).toBe('SentryStagingTestError');
    expect((thrown as Error).message).toMatch(/Manual Sentry test from staging UI/);
    expect(ctx).toMatchObject({
      tags: { test: 'manual-staging', source: 'developer-options-button' },
      level: 'error',
    });
    expect(hoisted.flush).toHaveBeenCalledWith(2000);
  });
});
