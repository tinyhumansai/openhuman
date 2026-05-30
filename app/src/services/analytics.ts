/**
 * Analytics & Sentry service
 *
 * Initializes Sentry for error reporting and OpenPanel for anonymous
 * usage tracking. Both are gated on user analytics consent.
 *
 * Sentry privacy guarantees enforced in `beforeSend`:
 *   - No breadcrumbs, requests, extras, or arbitrary contexts (only OS /
 *     browser / device metadata kept)
 *   - No frame-level locals or source-context snippets
 *   - No PII — `user` is reduced to a stable anonymous id (or omitted)
 *   - `sendDefaultPii: false` (no IP, no cookies)
 *   - All breadcrumb-producing integrations disabled
 *
 * OpenPanel privacy guarantees:
 *   - Only page views and feature-engagement events from the allowlist are sent
 *   - No user content, messages, credentials, or PII is ever included
 */
import * as Sentry from '@sentry/react';

import { getCoreStateSnapshot } from '../lib/coreState/store';
import {
  APP_ENVIRONMENT,
  GA_MEASUREMENT_ID,
  IS_DEV,
  SENTRY_DSN,
  SENTRY_RELEASE,
  SENTRY_SMOKE_TEST,
} from '../utils/config';
import { CoreRpcError } from './coreRpcClient';

// ---------------------------------------------------------------------------
// Google Analytics 4 typings — raw gtag.js API
// ---------------------------------------------------------------------------

type GtagCommand = 'config' | 'event' | 'set' | 'js';
interface GtagFn {
  (...args: [GtagCommand, ...unknown[]]): void;
}

// ---------------------------------------------------------------------------
// OpenPanel typings — raw script injection API
// ---------------------------------------------------------------------------

type OpMethod = 'init' | 'track' | 'identify' | 'increment' | 'decrement' | 'clear' | 'alias';
interface OpFn {
  (...args: [OpMethod, ...unknown[]]): void;
  q?: unknown[];
}

declare global {
  interface Window {
    dataLayer: unknown[];
    gtag: GtagFn;
    op: OpFn;
  }
}

const OPENPANEL_CLIENT_ID = 'e9c996d5-497f-4eec-9bde-630019ad525b';
const OPENPANEL_API_URL = 'https://panel.tinyhumans.ai/api';

// ---------------------------------------------------------------------------
// Module-level state
// ---------------------------------------------------------------------------

let gaInitialized = false;
let opInitialized = false;

/**
 * Shadow of the user's analytics consent state. Kept in sync by
 * `syncAnalyticsConsent`. Default: `false` (deny until explicitly allowed).
 */
let analyticsEnabled = false;

/**
 * Allowlist of event names that may be sent to OpenPanel.
 *
 * Keeping an explicit allowlist prevents accidentally forwarding internal
 * debug names or future ad-hoc calls that could carry sensitive information.
 * Any `trackEvent` call with a name not in this set is dropped and a warning
 * is logged.
 */
export const ALLOWED_EVENTS = new Set([
  'app_open',
  'onboarding_start',
  'onboarding_step_complete',
  'onboarding_complete',
  'account_connect_start',
  'account_connect_success',
  'chat_message_sent',
  'skill_install',
  'skill_uninstall',
]);

/** Check if the current user has opted into analytics. */
export function isAnalyticsEnabled(): boolean {
  return getCoreStateSnapshot().snapshot.analyticsEnabled;
}

/**
 * Cross-realm-safe check for a `CoreRpcError` with `kind === 'timeout'`.
 * `instanceof` can fail across module scopes (test harness, dynamic import,
 * Vitest module isolation), so also accept a duck-typed match on `name`
 * and `kind`. Used by the Sentry `beforeSend` filter to drop the
 * OPENHUMAN-REACT-15/11/10/12/Z/Y family at the source.
 */
function isCoreRpcTimeoutError(err: unknown): boolean {
  if (err instanceof CoreRpcError) return err.kind === 'timeout';
  if (typeof err !== 'object' || err === null) return false;
  const candidate = err as { name?: unknown; kind?: unknown };
  return candidate.name === 'CoreRpcError' && candidate.kind === 'timeout';
}

export function initSentry(): void {
  if (!SENTRY_DSN) return;

  Sentry.init({
    dsn: SENTRY_DSN,
    environment: APP_ENVIRONMENT,
    // Canonical release tag shared with the Tauri shell (see
    // `app/src-tauri/src/lib.rs::build_sentry_release_tag`) and the Vite
    // source-map upload (see `@sentry/vite-plugin` in app/vite.config.ts)
    // so events from every surface group under the same release.
    release: SENTRY_RELEASE,
    enabled: !IS_DEV,

    // Privacy: disable EVERYTHING that could leak sensitive state.
    replaysSessionSampleRate: 0,
    replaysOnErrorSampleRate: 0,
    tracesSampleRate: 0,
    defaultIntegrations: false,
    integrations: [
      Sentry.functionToStringIntegration(),
      Sentry.linkedErrorsIntegration(),
      Sentry.dedupeIntegration(),
      Sentry.browserApiErrorsIntegration(),
      Sentry.globalHandlersIntegration(),
      // #1403: production events were missing `os.name` / `browser.name` /
      // `device.family` because Sentry derives those by parsing the
      // User-Agent header server-side, and `defaultIntegrations: false`
      // (above) drops the integration that attaches `event.request.headers`.
      // Re-include it explicitly so platform context comes back. `beforeSend`
      // narrows what survives from the request envelope (headers only, UA
      // only) to keep this aligned with the privacy contract.
      Sentry.httpContextIntegration(),
    ],
    sendDefaultPii: false,

    beforeSend(event, hint) {
      // Drop noisy local-AbortController RPC timeouts at the source so a
      // missed `.catch()` at a future call site cannot regress the
      // OPENHUMAN-REACT-15/11/10/12/Z/Y family. Sister to the Rust-side
      // `is_session_expired_event` filter / loopback classifier in PR #2063.
      // Cross-realm-safe: also accept a non-instanceof match on the
      // class name + kind (test harness can construct CoreRpcError in a
      // different module scope).
      const original = hint?.originalException as unknown;
      if (isCoreRpcTimeoutError(original)) {
        return null;
      }

      // Always allow the smoke-test event through so pipeline validation works
      // even when the user hasn't opted into analytics yet on first boot.
      const isSmokeTest = event.message === 'react-sentry-smoke-test';
      // Manual staging test events fired from the Developer Options button
      // (#1072) bypass the consent gate so QA can validate the pipeline
      // without needing to flip user-facing analytics first. The bypass is
      // *also* gated on APP_ENVIRONMENT so a stray `manual-staging` tag in
      // production (whether accidental or malicious) cannot exfiltrate an
      // event past the consent gate — the only legitimate caller in this
      // codebase is `triggerSentryTestEvent` and it itself refuses to fire
      // outside staging.
      const isManualTest = APP_ENVIRONMENT === 'staging' && event.tags?.test === 'manual-staging';
      // Drop events when the user hasn't opted into analytics.
      if (!isSmokeTest && !isManualTest && !isAnalyticsEnabled()) return null;

      // Strip anything that could carry Redux / localStorage / request bodies.
      event.breadcrumbs = [];
      // Keep only the User-Agent header so Sentry's server-side relay can
      // populate `os` / `browser` / `device` contexts (#1403). Drop URL,
      // query string, cookies, and request body — anything that could leak
      // user content or session state.
      const ua = (event.request?.headers as Record<string, string> | undefined)?.['User-Agent'];
      event.request = ua ? { headers: { 'User-Agent': ua } } : undefined;
      delete event.extra;
      event.contexts = {
        os: event.contexts?.os,
        browser: event.contexts?.browser,
        device: event.contexts?.device,
      };

      // Tag with surface so events filter cleanly inside `openhuman-react`.
      event.tags = { ...(event.tags ?? {}), surface: 'react' };

      // Strip PII; keep a stable anonymous user id only.
      const userId = getCoreStateSnapshot().snapshot.currentUser?._id;
      event.user = userId ? { id: userId } : undefined;

      // Strip frame-level local variables and source context — never send
      // raw source snippets or live variable values to the dashboard.
      if (event.exception?.values) {
        for (const v of event.exception.values) {
          if (v.stacktrace?.frames) {
            for (const f of v.stacktrace.frames) {
              delete f.vars;
              delete f.context_line;
              delete f.pre_context;
              delete f.post_context;
            }
          }
          if (v.mechanism) {
            delete v.mechanism.data;
          }
        }
      }

      return event;
    },

    // Ignore common non-actionable errors.
    ignoreErrors: ['ResizeObserver loop', 'Network request failed', 'Load failed', 'AbortError'],
  });

  // Optional smoke trigger for verifying the pipeline end-to-end. Set
  // `VITE_SENTRY_SMOKE_TEST=true` for one build (or in `.env.local` for
  // local verification) and the next initSentry call will fire a test
  // message before returning. No-op when unset. The smoke event bypasses
  // the analytics-consent gate in `beforeSend` so it reaches Sentry even
  // on a fresh install where consent hasn't been granted yet.
  if (SENTRY_SMOKE_TEST) {
    Sentry.captureMessage('react-sentry-smoke-test', 'info');
  }
}

/**
 * Re-sync Sentry's enabled state after the user changes their consent.
 * Called from onboarding and settings.
 *
 * `beforeSend` reads `isAnalyticsEnabled()` on every event, so toggling
 * consent takes effect immediately for new errors. Flush pending events
 * on opt-out so anything already in flight respects the previous state.
 *
 * Also updates the module-level `gaEnabled` flag so `trackPageView` and
 * `trackEvent` respect the new consent state without reinitializing GA.
 */
export function syncAnalyticsConsent(enabled: boolean): void {
  const client = Sentry.getClient();
  if (client && !enabled) {
    void Sentry.flush(2000);
  }

  analyticsEnabled = enabled;
  if (gaInitialized || opInitialized) {
    console.debug(`[analytics] consent updated: enabled=${enabled}`);
  }
}

// ---------------------------------------------------------------------------
// Analytics — public API (GA4 + OpenPanel, both fire on every call)
// ---------------------------------------------------------------------------

function initGoogleAnalytics(): void {
  if (gaInitialized || !GA_MEASUREMENT_ID) return;
  try {
    window.dataLayer = window.dataLayer || [];
    window.gtag = function gtag(...args: [GtagCommand, ...unknown[]]) {
      window.dataLayer.push(args);
    };
    window.gtag('js', new Date());
    window.gtag('config', GA_MEASUREMENT_ID, {
      send_page_view: false,
      allow_ad_personalization_signals: false,
    });

    const script = document.createElement('script');
    script.async = true;
    script.src = `https://www.googletagmanager.com/gtag/js?id=${GA_MEASUREMENT_ID}`;
    document.head.appendChild(script);

    gaInitialized = true;
    console.debug('[analytics] GA initialized (gtag.js)', { measurementId: GA_MEASUREMENT_ID });
  } catch (err) {
    console.warn('[analytics] GA initialization failed:', err);
  }
}

function initOpenPanel(): void {
  if (opInitialized) return;
  try {
    window.op =
      window.op ||
      (function (this: void) {
        const n: unknown[] = [];
        return new Proxy(
          function (this: void) {
            if (arguments.length) n.push([].slice.call(arguments));
          } as unknown as OpFn,
          {
            get(_t: unknown, r: string) {
              if (r === 'q') return n;
              return function () {
                n.push([r].concat([].slice.call(arguments)));
              };
            },
            has(_t: unknown, r: string) {
              return r === 'q';
            },
          }
        );
      })();

    window.op('init', {
      apiUrl: OPENPANEL_API_URL,
      clientId: OPENPANEL_CLIENT_ID,
      trackScreenViews: false,
      trackOutgoingLinks: false,
      trackAttributes: false,
    });

    const script = document.createElement('script');
    script.defer = true;
    script.async = true;
    script.src = 'https://openpanel.dev/op1.js';
    document.head.appendChild(script);

    opInitialized = true;
    console.debug('[analytics] OpenPanel initialized', { clientId: OPENPANEL_CLIENT_ID });
  } catch (err) {
    console.warn('[analytics] OpenPanel initialization failed:', err);
  }
}

/**
 * Initialize all analytics providers (GA4 + OpenPanel).
 * Idempotent — each provider initializes at most once.
 */
export function initGA(): void {
  initGoogleAnalytics();
  initOpenPanel();
  analyticsEnabled = isAnalyticsEnabled();
}

/**
 * Send an anonymous page view to all initialized providers.
 */
export function trackPageView(path: string): void {
  if ((!gaInitialized && !opInitialized) || !analyticsEnabled) return;
  console.debug('[analytics] trackPageView', { path });
  if (gaInitialized) window.gtag('event', 'page_view', { page_path: path });
  if (opInitialized) window.op('track', 'screen_view', { page: path });
}

/**
 * Send an anonymous feature-engagement event to all initialized providers.
 *
 * Event names must appear in `ALLOWED_EVENTS`. Calls with unlisted names
 * are dropped and a console warning is emitted.
 */
export function trackEvent(
  eventName: string,
  params?: Record<string, string | number | boolean>
): void {
  if ((!gaInitialized && !opInitialized) || !analyticsEnabled) return;

  if (!ALLOWED_EVENTS.has(eventName)) {
    console.warn(
      `[analytics] trackEvent dropped — '${eventName}' is not in ALLOWED_EVENTS allowlist`
    );
    return;
  }

  console.debug('[analytics] trackEvent', { eventName, params });
  if (gaInitialized) window.gtag('event', eventName, params);
  if (opInitialized) window.op('track', eventName, params);
}

/**
 * Fire a manual diagnostic event for issue #1072: a staging-only "Trigger
 * Sentry Test" button uses this to validate the React → Sentry pipeline
 * end-to-end after a config change. Tagged so `beforeSend` lets it through
 * regardless of analytics consent, and so it's trivial to filter on the
 * dashboard side. Returns the event id Sentry assigns (or `undefined` if
 * Sentry is disabled in this build).
 */
export async function triggerSentryTestEvent(): Promise<string | undefined> {
  // Fail-fast outside staging. The UI button is only rendered when
  // `APP_ENVIRONMENT === 'staging'`, but this guard exists as defense in
  // depth so a programmatic caller (a stray import, a future refactor)
  // cannot fire diagnostic events from production. `beforeSend` already
  // re-checks the same gate before applying the consent bypass.
  if (APP_ENVIRONMENT !== 'staging') {
    console.warn(
      `[sentry-test] refusing to fire test event outside staging (APP_ENVIRONMENT=${APP_ENVIRONMENT})`
    );
    return undefined;
  }

  const client = Sentry.getClient();
  if (!client) {
    console.warn('[sentry-test] Sentry client not initialized — DSN missing or dev build');
    return undefined;
  }

  // Constant message so Sentry's default grouping algorithm collapses every
  // QA click into one issue (with N events) instead of one issue per click.
  // Per-click timing goes through `extra` so it's still visible on each
  // event but doesn't influence the fingerprint.
  const stamp = new Date().toISOString();
  const error = new Error('Manual Sentry test from staging UI');
  error.name = 'SentryStagingTestError';

  const eventId = Sentry.captureException(error, {
    tags: { test: 'manual-staging', source: 'developer-options-button' },
    extra: { triggered_at: stamp },
    level: 'error',
  });

  console.info('[sentry-test] captureException eventId=', eventId);
  // Surface flush timeouts as failures: a `false` here means the event
  // queue did not drain within 2s, so the network round-trip to Sentry is
  // unconfirmed. For a *diagnostic* tool, returning a successful-looking
  // eventId in that case would be a lie.
  const flushed = await Sentry.flush(2000);
  if (!flushed) {
    throw new Error(
      'Sentry.flush(2000) timed out — event may not have reached Sentry. ' +
        'Check network / DSN / Sentry status before retrying.'
    );
  }
  return eventId;
}
