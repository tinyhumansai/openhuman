import { beforeEach, describe, expect, test, vi } from 'vitest';

// Hoisted mocks so tests can swap return values per case.
const hoisted = vi.hoisted(() => ({
  getClient: vi.fn(),
  captureException: vi.fn(),
  captureMessage: vi.fn(),
  flush: vi.fn(() => Promise.resolve(true)),
  init: vi.fn(),
  // Integration stubs — these aren't introspected, just need to exist so
  // `Sentry.init()` accepts the integrations array without throwing.
  functionToStringIntegration: vi.fn(() => ({})),
  linkedErrorsIntegration: vi.fn(() => ({})),
  dedupeIntegration: vi.fn(() => ({})),
  browserApiErrorsIntegration: vi.fn(() => ({})),
  globalHandlersIntegration: vi.fn(() => ({})),
  analyticsEnabled: false,
}));

vi.mock('@sentry/react', () => ({
  getClient: hoisted.getClient,
  captureException: hoisted.captureException,
  captureMessage: hoisted.captureMessage,
  flush: hoisted.flush,
  init: hoisted.init,
  functionToStringIntegration: hoisted.functionToStringIntegration,
  linkedErrorsIntegration: hoisted.linkedErrorsIntegration,
  dedupeIntegration: hoisted.dedupeIntegration,
  browserApiErrorsIntegration: hoisted.browserApiErrorsIntegration,
  globalHandlersIntegration: hoisted.globalHandlersIntegration,
}));

// `initSentry()` reads `getCoreStateSnapshot().snapshot.analyticsEnabled` to
// decide whether non-test events get dropped. Mock it so each test can flip
// consent without instantiating the real Redux/persistence stack.
vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: () => ({
    snapshot: { analyticsEnabled: hoisted.analyticsEnabled, currentUser: null },
  }),
}));

// `initSentry()` only does anything when SENTRY_DSN is truthy and IS_DEV is
// false. Mock the whole config module so we control both gates.
vi.mock('../../utils/config', () => ({
  APP_ENVIRONMENT: 'staging',
  IS_DEV: false,
  SENTRY_DSN: 'https://abc@example.ingest.sentry.io/1',
  SENTRY_RELEASE: 'openhuman@test+abc',
  SENTRY_SMOKE_TEST: false,
}));

describe('triggerSentryTestEvent', () => {
  beforeEach(() => {
    hoisted.getClient.mockReset();
    hoisted.captureException.mockReset();
    hoisted.flush.mockReset();
    hoisted.flush.mockReturnValue(Promise.resolve(true));
    hoisted.init.mockReset();
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

describe('initSentry beforeSend manual-staging bypass', () => {
  /** Capture the `beforeSend` callback that `initSentry` registers. */
  async function captureBeforeSend(): Promise<
    (event: Record<string, unknown>) => Record<string, unknown> | null
  > {
    hoisted.init.mockReset();
    const { initSentry } = await import('../analytics');
    initSentry();
    expect(hoisted.init).toHaveBeenCalledTimes(1);
    const opts = hoisted.init.mock.calls[0][0] as {
      beforeSend: (event: Record<string, unknown>) => Record<string, unknown> | null;
    };
    return opts.beforeSend.bind(opts);
  }

  beforeEach(() => {
    hoisted.analyticsEnabled = false;
  });

  test('drops events when consent is off and event is not test-tagged', async () => {
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({ message: 'something blew up', tags: {}, contexts: {} });
    expect(result).toBeNull();
  });

  test('lets manual-staging tagged events through even without consent', async () => {
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({
      message: 'something blew up',
      tags: { test: 'manual-staging' },
      breadcrumbs: [{ message: 'should-be-stripped' }],
      request: { url: 'https://api.example.com/secret' },
      extra: { token: 'redacted-please' },
      contexts: { os: { name: 'macOS' }, app: { build: '123' } },
    }) as Record<string, unknown> | null;

    expect(result).not.toBeNull();
    // PII / breadcrumbs / request body / extras must all be stripped.
    expect((result as { breadcrumbs: unknown[] }).breadcrumbs).toEqual([]);
    expect(result).not.toHaveProperty('request');
    expect(result).not.toHaveProperty('extra');
    // `app` context is stripped — only os/browser/device kept.
    expect((result as { contexts: Record<string, unknown> }).contexts).not.toHaveProperty('app');
    expect((result as { contexts: Record<string, unknown> }).contexts).toHaveProperty('os');
    // `surface=react` is added so the dashboard can filter cleanly.
    expect((result as { tags: Record<string, string> }).tags).toMatchObject({
      test: 'manual-staging',
      surface: 'react',
    });
  });

  test('still lets the smoke-test message through (existing behaviour)', async () => {
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({ message: 'react-sentry-smoke-test', tags: {}, contexts: {} });
    expect(result).not.toBeNull();
  });
});
