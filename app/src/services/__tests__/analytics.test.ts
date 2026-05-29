import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

// Hoisted mocks so tests can swap return values per case.
const hoisted = vi.hoisted(() => ({
  // Sentry stubs
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
  browserApiErrorsIntegration: vi.fn(() => ({ name: 'BrowserApiErrors' })),
  globalHandlersIntegration: vi.fn(() => ({ name: 'GlobalHandlers' })),
  httpContextIntegration: vi.fn(() => ({ name: 'HttpContext' })),
  // Config state
  analyticsEnabled: false,
  appEnvironment: 'staging' as 'staging' | 'production' | 'development',
  isDev: false,
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
  httpContextIntegration: hoisted.httpContextIntegration,
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
// false. Mock the whole config module so we control both gates. Use a
// getter for APP_ENVIRONMENT so tests can flip staging/production per-case
// to exercise the defense-in-depth gates added for the consent bypass.
// Getters for GA_MEASUREMENT_ID and IS_DEV allow per-test overrides.
vi.mock('../../utils/config', () => ({
  get APP_ENVIRONMENT() {
    return hoisted.appEnvironment;
  },
  get IS_DEV() {
    return hoisted.isDev;
  },
  GA_MEASUREMENT_ID: 'G-TEST12345',
  SENTRY_DSN: 'https://abc@example.ingest.sentry.io/1',
  SENTRY_RELEASE: 'openhuman@test+abc',
  SENTRY_SMOKE_TEST: false,
  // analytics.ts now imports CoreRpcError from coreRpcClient, whose
  // dependency chain reads CORE_RPC_URL and CORE_RPC_TIMEOUT_MS. Provide
  // stub values so the module graph loads under this mock.
  CORE_RPC_URL: 'http://127.0.0.1:7788/rpc',
  CORE_RPC_TIMEOUT_MS: 30000,
}));

describe('triggerSentryTestEvent', () => {
  beforeEach(() => {
    hoisted.getClient.mockReset();
    hoisted.captureException.mockReset();
    hoisted.flush.mockReset();
    hoisted.flush.mockReturnValue(Promise.resolve(true));
    hoisted.init.mockReset();
    hoisted.appEnvironment = 'staging';
  });

  test('refuses to fire outside staging (defense in depth)', async () => {
    hoisted.appEnvironment = 'production';
    hoisted.getClient.mockReturnValue({});
    const { triggerSentryTestEvent } = await import('../analytics');

    const result = await triggerSentryTestEvent();

    expect(result).toBeUndefined();
    expect(hoisted.captureException).not.toHaveBeenCalled();
    expect(hoisted.flush).not.toHaveBeenCalled();
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
    hoisted.flush.mockReturnValue(Promise.resolve(true));
    const { triggerSentryTestEvent } = await import('../analytics');

    const result = await triggerSentryTestEvent();

    expect(result).toBe('event-id-abc');
    expect(hoisted.captureException).toHaveBeenCalledTimes(1);

    const [thrown, ctx] = hoisted.captureException.mock.calls[0];
    expect(thrown).toBeInstanceOf(Error);
    expect((thrown as Error).name).toBe('SentryStagingTestError');
    // Message is constant so Sentry groups every test click into one issue.
    expect((thrown as Error).message).toBe('Manual Sentry test from staging UI');
    expect(ctx).toMatchObject({
      tags: { test: 'manual-staging', source: 'developer-options-button' },
      level: 'error',
    });
    // Per-click timing rides on `extra`, not in the message — high cardinality
    // there would explode tag indexes and break grouping.
    expect((ctx as { extra: { triggered_at: string } }).extra.triggered_at).toMatch(
      /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/
    );
    expect(hoisted.flush).toHaveBeenCalledWith(2000);
  });

  test('throws when flush times out so the UI surfaces an error', async () => {
    hoisted.getClient.mockReturnValue({});
    hoisted.captureException.mockReturnValue('event-id-stuck');
    hoisted.flush.mockReturnValue(Promise.resolve(false));
    const { triggerSentryTestEvent } = await import('../analytics');

    await expect(triggerSentryTestEvent()).rejects.toThrow(/timed out/i);
  });
});

describe('initSentry beforeSend manual-staging bypass', () => {
  /** Capture the `beforeSend` callback that `initSentry` registers. */
  async function captureBeforeSend(): Promise<
    (
      event: Record<string, unknown>,
      hint?: { originalException?: unknown }
    ) => Record<string, unknown> | null
  > {
    hoisted.init.mockReset();
    const { initSentry } = await import('../analytics');
    initSentry();
    expect(hoisted.init).toHaveBeenCalledTimes(1);
    const opts = hoisted.init.mock.calls[0][0] as {
      beforeSend: (
        event: Record<string, unknown>,
        hint?: { originalException?: unknown }
      ) => Record<string, unknown> | null;
    };
    return opts.beforeSend.bind(opts);
  }

  beforeEach(() => {
    hoisted.analyticsEnabled = false;
    hoisted.appEnvironment = 'staging';
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
      request: {
        url: 'https://api.example.com/secret',
        cookies: 'session=abc',
        data: { body: 'redacted' },
        headers: { 'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)' },
      },
      extra: { token: 'redacted-please' },
      contexts: { os: { name: 'macOS' }, app: { build: '123' } },
    }) as Record<string, unknown> | null;

    expect(result).not.toBeNull();
    // PII / breadcrumbs / request body / extras must all be stripped.
    expect((result as { breadcrumbs: unknown[] }).breadcrumbs).toEqual([]);
    // Request envelope is narrowed to the User-Agent header only — keeping
    // it lets Sentry's relay populate os/browser/device (#1403); URL,
    // cookies, and body are dropped.
    const req = (result as { request?: { headers?: Record<string, string>; url?: string } })
      .request;
    expect(req?.headers).toEqual({ 'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)' });
    expect(req).not.toHaveProperty('url');
    expect(req).not.toHaveProperty('cookies');
    expect(req).not.toHaveProperty('data');
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

  test('drops CoreRpcError with kind=timeout via the originalException hint', async () => {
    // Regression for OPENHUMAN-REACT-15/11/10/12/Z/Y: a missed `.catch()` at
    // any `await callCoreRpc(...)` chain in the team panels surfaced as an
    // unhandled rejection captured by `auto.browser.global_handlers`. Even
    // with .catch() landed, future call sites must not regress the family
    // — this filter is the last line of defense.
    hoisted.analyticsEnabled = true; // consent on so non-test events normally pass.
    const beforeSend = await captureBeforeSend();
    const { CoreRpcError } = await import('../coreRpcClient');
    const timeoutErr = new CoreRpcError(
      'Core RPC openhuman.team_list_teams timed out after 30000ms',
      'timeout'
    );

    const result = beforeSend(
      { message: 'CoreRpcError', tags: {}, contexts: {} },
      { originalException: timeoutErr }
    );
    expect(result).toBeNull();
  });

  test('drops cross-realm CoreRpcError-shaped timeouts (name + kind match)', async () => {
    // Test harnesses and dynamic imports can construct CoreRpcError in a
    // separate module scope where `instanceof` fails. The filter must still
    // demote them.
    hoisted.analyticsEnabled = true;
    const beforeSend = await captureBeforeSend();
    const fakeErr = Object.assign(new Error('Core RPC X timed out after 30000ms'), {
      name: 'CoreRpcError',
      kind: 'timeout',
    });

    const result = beforeSend(
      { message: 'CoreRpcError', tags: {}, contexts: {} },
      { originalException: fakeErr }
    );
    expect(result).toBeNull();
  });

  test('lets non-timeout CoreRpcError shapes through (transport, auth_expired, …)', async () => {
    hoisted.analyticsEnabled = true;
    const beforeSend = await captureBeforeSend();
    const { CoreRpcError } = await import('../coreRpcClient');
    const transportErr = new CoreRpcError('error sending request', 'transport');

    const result = beforeSend(
      { message: 'CoreRpcError', tags: {}, contexts: {} },
      { originalException: transportErr }
    );
    // Transport errors are still worth seeing — only the local 30s
    // AbortController shape gets demoted at the source.
    expect(result).not.toBeNull();
  });

  test('forwards release tag and registers httpContextIntegration (#1403)', async () => {
    // Regression for #1403: production events arrived in Sentry with no
    // `release` tag and no `os` context. The release must reach Sentry.init
    // verbatim from `SENTRY_RELEASE`, and `httpContextIntegration` must be
    // present so the User-Agent header is attached and the relay can derive
    // `os` / `browser` / `device` server-side.
    hoisted.init.mockReset();
    const { initSentry } = await import('../analytics');
    initSentry();

    const opts = hoisted.init.mock.calls[0][0] as {
      release: string;
      tracesSampleRate: number;
      replaysSessionSampleRate: number;
      replaysOnErrorSampleRate: number;
      integrations: Array<{ name?: string }>;
    };
    expect(opts.release).toBe('openhuman@test+abc');
    expect(opts.tracesSampleRate).toBe(0);
    expect(opts.replaysSessionSampleRate).toBe(0);
    expect(opts.replaysOnErrorSampleRate).toBe(0);
    const names = opts.integrations.map(i => i.name).filter(Boolean);
    expect(names).toContain('HttpContext');
  });

  test('keeps os/browser/device contexts and forwards them through beforeSend (#1403)', async () => {
    hoisted.analyticsEnabled = true; // consent on so beforeSend doesn't drop.
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({
      message: 'real prod error',
      tags: {},
      contexts: {
        os: { name: 'macOS', version: '14.0' },
        browser: { name: 'Chrome', version: '119' },
        device: { family: 'Mac' },
        // Anything other than os/browser/device must be dropped by the
        // privacy filter — if a future edit accidentally widens the
        // allowlist, this assertion fails.
        state: { redux: 'should-not-leak' },
      },
    }) as { contexts: Record<string, unknown> } | null;

    expect(result).not.toBeNull();
    expect(result!.contexts).toMatchObject({
      os: { name: 'macOS', version: '14.0' },
      browser: { name: 'Chrome', version: '119' },
      device: { family: 'Mac' },
    });
    expect(result!.contexts).not.toHaveProperty('state');
  });

  test('drops the entire request envelope when no User-Agent header is present', async () => {
    hoisted.analyticsEnabled = true;
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({
      message: 'no-ua event',
      tags: {},
      contexts: {},
      request: { url: 'https://leak/secret', headers: { 'X-Other': 'meh' } },
    }) as Record<string, unknown> | null;

    expect(result).not.toBeNull();
    expect(result!.request).toBeUndefined();
  });

  test('drops manual-staging tagged events in production even with the tag', async () => {
    // Defense in depth: a stray `tags.test = 'manual-staging'` in production
    // must NOT bypass the consent gate. Capture beforeSend in staging, then
    // flip APP_ENVIRONMENT to production *before* invoking it, so the
    // `isManualTest` check inside beforeSend re-reads the live value via the
    // mocked getter.
    const beforeSend = await captureBeforeSend();
    hoisted.appEnvironment = 'production';
    const result = beforeSend({
      message: 'pretending to be a test event',
      tags: { test: 'manual-staging' },
      contexts: {},
    });
    expect(result).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// GA4 tests
//
// Each test calls `vi.resetModules()` and re-imports `analytics` so that the
// module-level `gaInitialized` / `gaEnabled` flags start fresh. This mirrors
// the Sentry test pattern above (dynamic `import('../analytics')` per-test).
// ---------------------------------------------------------------------------

/** Stub for `document.createElement('script')` — captures the injected src. */
let createdScripts: Array<{ async: boolean; defer: boolean; src: string }> = [];
const originalCreateElement = document.createElement.bind(document);

/** Reset window.op and module state, return a fresh analytics module. */
async function freshAnalytics() {
  vi.resetModules();
  delete (window as unknown as Record<string, unknown>).op;
  createdScripts = [];
  vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
    if (tag === 'script') {
      const fake = { async: false, defer: false, src: '' } as unknown as HTMLScriptElement;
      createdScripts.push(fake as unknown as { async: boolean; defer: boolean; src: string });
      return fake;
    }
    return originalCreateElement(tag);
  });
  vi.spyOn(document.head, 'appendChild').mockImplementation((node: Node) => node);
  return import('../analytics');
}

describe('initGA (OpenPanel)', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = false;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('injects both gtag.js and op1.js scripts', async () => {
    hoisted.analyticsEnabled = true;
    const { initGA } = await freshAnalytics();
    initGA();
    expect(createdScripts).toHaveLength(2);
    expect(createdScripts[0].src).toBe('https://www.googletagmanager.com/gtag/js?id=G-TEST12345');
    expect(createdScripts[1].src).toBe('https://openpanel.dev/op1.js');
    expect(window.gtag).toBeDefined();
    expect(window.op).toBeDefined();
  });

  test('is idempotent — second call does not inject additional scripts', async () => {
    hoisted.analyticsEnabled = true;
    const { initGA } = await freshAnalytics();
    initGA();
    initGA();
    expect(createdScripts).toHaveLength(2);
  });
});

describe('trackPageView (OpenPanel)', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = true;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('sends a screen_view event when consent is on', async () => {
    const { initGA, trackPageView } = await freshAnalytics();
    initGA();
    const opSpy = vi.spyOn(window, 'op');
    trackPageView('/home');
    expect(opSpy).toHaveBeenCalledWith('track', 'screen_view', { page: '/home' });
  });

  test('is a no-op when consent is off', async () => {
    const { initGA, syncAnalyticsConsent, trackPageView } = await freshAnalytics();
    initGA();
    const opSpy = vi.spyOn(window, 'op');
    syncAnalyticsConsent(false);
    trackPageView('/home');
    expect(opSpy).not.toHaveBeenCalled();
  });

  test('is a no-op when OpenPanel was never initialized', async () => {
    const { trackPageView } = await freshAnalytics();
    trackPageView('/home');
    expect(window.op).toBeUndefined();
  });
});

describe('trackEvent (OpenPanel)', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = true;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('sends allowed events with correct params', async () => {
    const { initGA, trackEvent } = await freshAnalytics();
    initGA();
    const opSpy = vi.spyOn(window, 'op');
    trackEvent('app_open', { version: '1.0.0' });
    expect(opSpy).toHaveBeenCalledWith('track', 'app_open', { version: '1.0.0' });
  });

  test('drops events not in the allowlist and logs a warning', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const { initGA, trackEvent } = await freshAnalytics();
    initGA();
    const opSpy = vi.spyOn(window, 'op');
    trackEvent('internal_debug_event');
    const trackCalls = opSpy.mock.calls.filter(
      c => c[0] === 'track' && c[1] === 'internal_debug_event'
    );
    expect(trackCalls).toHaveLength(0);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('internal_debug_event'));
    warnSpy.mockRestore();
  });

  test('is a no-op when consent is off', async () => {
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    const opSpy = vi.spyOn(window, 'op');
    syncAnalyticsConsent(false);
    trackEvent('app_open');
    expect(opSpy).not.toHaveBeenCalled();
  });
});

describe('syncAnalyticsConsent OpenPanel integration', () => {
  beforeEach(() => {
    hoisted.getClient.mockReset();
    hoisted.flush.mockReset();
    hoisted.flush.mockReturnValue(Promise.resolve(true));
    hoisted.analyticsEnabled = true;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test('syncAnalyticsConsent(false) prevents subsequent events', async () => {
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    const opSpy = vi.spyOn(window, 'op');
    syncAnalyticsConsent(false);
    trackEvent('app_open');
    const trackCalls = opSpy.mock.calls.filter(c => c[0] === 'track');
    expect(trackCalls).toHaveLength(0);
  });

  test('syncAnalyticsConsent(true) re-enables events after disable', async () => {
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    syncAnalyticsConsent(false);
    syncAnalyticsConsent(true);
    const opSpy = vi.spyOn(window, 'op');
    trackEvent('app_open');
    expect(opSpy).toHaveBeenCalledWith('track', 'app_open', undefined);
  });
});
