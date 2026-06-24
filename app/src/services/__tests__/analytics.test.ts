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
  inboundFiltersIntegration: vi.fn(() => ({ name: 'InboundFilters' })),
  functionToStringIntegration: vi.fn(() => ({})),
  linkedErrorsIntegration: vi.fn(() => ({})),
  dedupeIntegration: vi.fn(() => ({})),
  browserApiErrorsIntegration: vi.fn(() => ({ name: 'BrowserApiErrors' })),
  globalHandlersIntegration: vi.fn(() => ({ name: 'GlobalHandlers' })),
  httpContextIntegration: vi.fn(() => ({ name: 'HttpContext' })),
  // Config state
  analyticsEnabled: false,
  appEnvironment: 'staging' as 'staging' | 'production' | 'development',
  currentUserId: null as string | null,
  isDev: false,
}));

vi.mock('@sentry/react', () => ({
  getClient: hoisted.getClient,
  captureException: hoisted.captureException,
  captureMessage: hoisted.captureMessage,
  flush: hoisted.flush,
  init: hoisted.init,
  inboundFiltersIntegration: hoisted.inboundFiltersIntegration,
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
    snapshot: {
      analyticsEnabled: hoisted.analyticsEnabled,
      currentUser: hoisted.currentUserId ? { _id: hoisted.currentUserId } : null,
    },
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
  APP_BINARY_VERSION: '0.57.4',
  APP_VERSION: '0.57.4',
  BUILD_SHA: 'abc123',
  CORE_CARGO_VERSION: '0.57.4',
  OPENPANEL_API_URL: 'https://panel.tinyhumans.ai/api',
  OPENPANEL_CLIENT_ID: 'e9c996d5-497f-4eec-9bde-630019ad525b',
  SENTRY_DSN: 'https://abc@example.ingest.sentry.io/1',
  SENTRY_RELEASE: 'openhuman@test+abc',
  SENTRY_SMOKE_TEST: false,
  SUPPORT_URL: 'https://support.example/help',
  TAURI_CARGO_VERSION: '0.57.4',
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
    hoisted.currentUserId = null;
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
    hoisted.currentUserId = null;
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

  test('seeds a support_url tag keyed on the event id (#3980)', async () => {
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({
      message: 'react-sentry-smoke-test', // bypasses consent gate
      event_id: 'deadbeefcafe',
      tags: {},
      contexts: {},
    }) as { tags: Record<string, string> } | null;
    expect(result).not.toBeNull();
    // Set as a TAG (extras are scrubbed); carries only the event's own id.
    expect(result?.tags.support_url).toBe('https://support.example/help?ref=deadbeefcafe');
    // Extras still dropped — the support link must not reintroduce a leak path.
    expect(result).not.toHaveProperty('extra');
  });

  test('omits support_url when the event has no id', async () => {
    const beforeSend = await captureBeforeSend();
    const result = beforeSend({ message: 'react-sentry-smoke-test', tags: {}, contexts: {} }) as {
      tags: Record<string, string>;
    } | null;
    expect(result).not.toBeNull();
    expect(result?.tags).not.toHaveProperty('support_url');
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

  test('registers inboundFiltersIntegration so ignoreErrors is not inert (#3963)', async () => {
    // Regression for #3963: `defaultIntegrations: false` drops the integration
    // that consumes the top-level `ignoreErrors` option. The curated list must
    // re-include `inboundFiltersIntegration`, otherwise the intended
    // `ResizeObserver loop` / network-noise filters silently leak to Sentry.
    // Asserting both the integration AND the non-empty filter list guards the
    // coupling — dropping either re-opens the flood.
    hoisted.init.mockReset();
    hoisted.inboundFiltersIntegration.mockClear();
    const { initSentry } = await import('../analytics');
    initSentry();

    const opts = hoisted.init.mock.calls[0][0] as {
      integrations: Array<{ name?: string }>;
      ignoreErrors: string[];
    };
    expect(hoisted.inboundFiltersIntegration).toHaveBeenCalledTimes(1);
    const names = opts.integrations.map(i => i.name).filter(Boolean);
    expect(names).toContain('InboundFilters');
    expect(opts.ignoreErrors).toEqual([
      'ResizeObserver loop',
      'Network request failed',
      'Load failed',
      'AbortError',
    ]);
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

/** Reset module state and return a fresh analytics module. */
async function freshAnalytics() {
  vi.resetModules();
  createdScripts = [];
  delete (window as Partial<Window>).gtag;
  delete (window as Partial<Window>).dataLayer;
  vi.stubGlobal(
    'fetch',
    vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve('{"deviceId":"test-device","sessionId":"test-session"}'),
      })
    )
  );
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

function openPanelPayload(callIndex = 0) {
  const body = String((fetch as ReturnType<typeof vi.fn>).mock.calls[callIndex][1].body);
  return JSON.parse(body) as {
    type: string;
    payload: {
      name: string;
      profileId?: string;
      properties: Record<string, string | number | boolean>;
    };
  };
}

function expectAnalyticsContext(properties: Record<string, string | number | boolean>) {
  expect(properties).toMatchObject({
    app_version: '0.57.4',
    binary_version: '0.57.4',
    core_cargo_version: '0.57.4',
    tauri_cargo_version: '0.57.4',
    release: 'openhuman@test+abc',
    build_sha: 'abc123',
    app_environment: 'staging',
  });
}

describe('initGA (OpenPanel)', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = false;
    hoisted.appEnvironment = 'staging';
    hoisted.currentUserId = null;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test('injects gtag.js and initializes OpenPanel direct ingestion without an external script', async () => {
    hoisted.analyticsEnabled = true;
    const { initGA } = await freshAnalytics();
    initGA();
    expect(createdScripts).toHaveLength(1);
    expect(createdScripts[0].src).toBe('https://www.googletagmanager.com/gtag/js?id=G-TEST12345');
    expect(window.gtag).toBeDefined();
  });

  test('does not initialize vendors until consent is enabled', async () => {
    hoisted.analyticsEnabled = false;
    const { initGA } = await freshAnalytics();
    initGA();
    expect(createdScripts).toHaveLength(0);
    expect(window.gtag).toBeUndefined();
  });

  test('initializes vendors when consent flips on after startup', async () => {
    hoisted.analyticsEnabled = false;
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    syncAnalyticsConsent(true);
    trackEvent('app_open');
    expect(createdScripts).toHaveLength(1);
    expect(fetch).toHaveBeenCalledWith(
      'https://panel.tinyhumans.ai/api/track',
      expect.objectContaining({
        method: 'POST',
        body: expect.stringContaining('"name":"app_open"'),
      })
    );
  });

  test('is idempotent — second call does not inject additional scripts', async () => {
    hoisted.analyticsEnabled = true;
    const { initGA } = await freshAnalytics();
    initGA();
    initGA();
    expect(createdScripts).toHaveLength(1);
  });
});

describe('trackPageView (OpenPanel)', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = true;
    hoisted.appEnvironment = 'staging';
    hoisted.currentUserId = null;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test('sends a screen_view event to OpenPanel when consent is on', async () => {
    const { initGA, trackPageView } = await freshAnalytics();
    document.title = 'OpenHuman Test';
    initGA();
    trackPageView('/home');
    expect(fetch).toHaveBeenCalledWith(
      'https://panel.tinyhumans.ai/api/track',
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({
          'openpanel-client-id': 'e9c996d5-497f-4eec-9bde-630019ad525b',
          'openpanel-sdk-name': 'openhuman-react',
        }),
        body: expect.stringContaining('"name":"screen_view"'),
        keepalive: true,
      })
    );
    expect(openPanelPayload().payload.properties).toMatchObject({
      page: '/home',
      user_id: '',
      page_hash: '',
      __path: '/home',
      __referrer: document.referrer,
      __title: 'OpenHuman Test',
    });
    expectAnalyticsContext(openPanelPayload().payload.properties);
    expect(openPanelPayload().payload.properties.__timestamp).toEqual(expect.any(String));
  });

  test('is a no-op when consent is off', async () => {
    const { initGA, syncAnalyticsConsent, trackPageView } = await freshAnalytics();
    initGA();
    syncAnalyticsConsent(false);
    trackPageView('/home');
    expect(fetch).not.toHaveBeenCalled();
  });

  test('is a no-op when OpenPanel was never initialized', async () => {
    const { trackPageView } = await freshAnalytics();
    trackPageView('/home');
    expect(fetch).not.toHaveBeenCalled();
  });
});

describe('trackEvent (OpenPanel)', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = true;
    hoisted.appEnvironment = 'staging';
    hoisted.currentUserId = null;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test('sends allowed events with correct params to OpenPanel', async () => {
    hoisted.currentUserId = 'user-123';
    const { initGA, trackEvent } = await freshAnalytics();
    initGA();
    trackEvent('app_open', { version: '1.0.0' });
    expect(fetch).toHaveBeenCalledWith(
      'https://panel.tinyhumans.ai/api/track',
      expect.objectContaining({
        method: 'POST',
        body: expect.stringContaining('"name":"app_open"'),
      })
    );
    expect(openPanelPayload().payload.properties).toMatchObject({
      version: '1.0.0',
      user_id: 'user-123',
      __path: window.location.pathname,
      __referrer: document.referrer,
    });
    expect(openPanelPayload().payload.profileId).toBe('user-123');
    expectAnalyticsContext(openPanelPayload().payload.properties);
    expect(openPanelPayload().payload.properties.__timestamp).toEqual(expect.any(String));
  });

  test('allows dedicated Tauri browser click events with provider-level metadata', async () => {
    const { initGA, trackEvent } = await freshAnalytics();
    initGA();
    trackEvent('tauri_browser_click', {
      surface: 'chat_right_sidebar',
      action: 'select_account',
      provider: 'slack',
      account_status: 'open',
    });

    expect(openPanelPayload().payload.name).toBe('tauri_browser_click');
    expect(openPanelPayload().payload.properties).toMatchObject({
      surface: 'chat_right_sidebar',
      action: 'select_account',
      provider: 'slack',
      account_status: 'open',
      user_id: '',
    });
  });

  test('redacts user_id from event debug logs', async () => {
    const debugSpy = vi.spyOn(console, 'debug').mockImplementation(() => undefined);
    hoisted.currentUserId = 'user-123';
    const { initGA, trackEvent } = await freshAnalytics();
    initGA();
    trackEvent('app_open', { version: '1.0.0' });
    const trackLog = debugSpy.mock.calls.find(call => call[0] === '[analytics] trackEvent');
    expect(trackLog?.[1]).toMatchObject({
      eventName: 'app_open',
      params: expect.not.objectContaining({ user_id: 'user-123' }),
    });
    debugSpy.mockRestore();
  });

  test('drops events not in the allowlist and logs a warning', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    const { initGA, trackEvent } = await freshAnalytics();
    initGA();
    trackEvent('internal_debug_event');
    expect(fetch).not.toHaveBeenCalled();
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('internal_debug_event'));
    warnSpy.mockRestore();
  });

  test('is a no-op when consent is off', async () => {
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    syncAnalyticsConsent(false);
    trackEvent('app_open');
    expect(fetch).not.toHaveBeenCalled();
  });
});

describe('startUiInteractionTracking', () => {
  beforeEach(() => {
    hoisted.analyticsEnabled = true;
    hoisted.appEnvironment = 'staging';
    hoisted.currentUserId = 'user-456';
    hoisted.isDev = false;
    window.location.hash = '#/home';
  });

  afterEach(() => {
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test('tracks delegated button clicks with stable scrubbed identifiers', async () => {
    const { initGA, startUiInteractionTracking } = await freshAnalytics();
    initGA();
    const stop = startUiInteractionTracking();
    const button = document.createElement('button');
    button.setAttribute('data-testid', 'thread-row-123456789-open');
    button.type = 'button';
    document.body.appendChild(button);

    button.click();

    expect(openPanelPayload().payload.name).toBe('ui_click');
    expect(openPanelPayload().payload.properties).toMatchObject({
      page: '/home',
      page_hash: '#/home',
      user_id: 'user-456',
      interaction_kind: 'click',
      element_tag: 'button',
      element_type: 'button',
      control_id: 'thread-row-:num-open',
    });
    expect(openPanelPayload().payload.profileId).toBe('user-456');
    expect(openPanelPayload().payload.properties.__path).toBe('/home');
    expectAnalyticsContext(openPanelPayload().payload.properties);
    stop();
  });

  test('tracks checkbox/radio/select style control changes without raw values', async () => {
    const { initGA, startUiInteractionTracking } = await freshAnalytics();
    initGA();
    const stop = startUiInteractionTracking();
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.name = 'analytics-enabled';
    document.body.appendChild(checkbox);

    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));

    expect(openPanelPayload().payload.name).toBe('ui_control_change');
    expect(openPanelPayload().payload.properties).toMatchObject({
      page: '/home',
      interaction_kind: 'change',
      element_tag: 'input',
      element_type: 'checkbox',
      control_id: 'analytics-enabled',
      control_state: 'checked',
    });
    stop();
  });

  test('tracks form submits by form identifier only', async () => {
    const { initGA, startUiInteractionTracking } = await freshAnalytics();
    initGA();
    const stop = startUiInteractionTracking();
    const form = document.createElement('form');
    form.setAttribute('data-testid', 'migration-form');
    document.body.appendChild(form);

    form.dispatchEvent(new SubmitEvent('submit', { bubbles: true, cancelable: true }));

    expect(openPanelPayload().payload.name).toBe('ui_form_submit');
    expect(openPanelPayload().payload.properties).toMatchObject({
      page: '/home',
      interaction_kind: 'submit',
      element_tag: 'form',
      control_id: 'migration-form',
    });
    stop();
  });

  test('does not track typed text inputs', async () => {
    const { initGA, startUiInteractionTracking } = await freshAnalytics();
    initGA();
    const stop = startUiInteractionTracking();
    const input = document.createElement('input');
    input.type = 'text';
    input.name = 'secret-user-text';
    document.body.appendChild(input);

    input.dispatchEvent(new Event('change', { bubbles: true }));

    expect(fetch).not.toHaveBeenCalled();
    stop();
  });
});

describe('syncAnalyticsConsent OpenPanel integration', () => {
  beforeEach(() => {
    hoisted.getClient.mockReset();
    hoisted.flush.mockReset();
    hoisted.flush.mockReturnValue(Promise.resolve(true));
    hoisted.analyticsEnabled = true;
    hoisted.appEnvironment = 'staging';
    hoisted.currentUserId = null;
    hoisted.isDev = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  test('syncAnalyticsConsent(false) prevents subsequent events', async () => {
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    syncAnalyticsConsent(false);
    trackEvent('app_open');
    expect(fetch).not.toHaveBeenCalled();
  });

  test('syncAnalyticsConsent(true) re-enables events after disable', async () => {
    const { initGA, syncAnalyticsConsent, trackEvent } = await freshAnalytics();
    initGA();
    syncAnalyticsConsent(false);
    syncAnalyticsConsent(true);
    trackEvent('app_open');
    expect(fetch).toHaveBeenCalledWith(
      'https://panel.tinyhumans.ai/api/track',
      expect.objectContaining({
        method: 'POST',
        body: expect.stringContaining('"name":"app_open"'),
      })
    );
  });
});
