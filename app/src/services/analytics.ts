/**
 * Analytics & Sentry service
 *
 * Initializes Sentry for the React frontend with auto-send semantics:
 * captured errors are sanitized in `beforeSend` and forwarded to Sentry,
 * gated only by user analytics consent.
 *
 * Privacy guarantees enforced in `beforeSend`:
 *   - No breadcrumbs, requests, extras, or arbitrary contexts (only OS /
 *     browser / device metadata kept)
 *   - No frame-level locals or source-context snippets
 *   - No PII — `user` is reduced to a stable anonymous id (or omitted)
 *   - `sendDefaultPii: false` (no IP, no cookies)
 *   - All breadcrumb-producing integrations disabled
 */
import * as Sentry from '@sentry/react';

import { getCoreStateSnapshot } from '../lib/coreState/store';
import {
  APP_ENVIRONMENT,
  IS_DEV,
  SENTRY_DSN,
  SENTRY_RELEASE,
  SENTRY_SMOKE_TEST,
} from '../utils/config';

/** Check if the current user has opted into analytics. */
export function isAnalyticsEnabled(): boolean {
  return getCoreStateSnapshot().snapshot.analyticsEnabled;
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
    ],
    sendDefaultPii: false,

    beforeSend(event) {
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
      delete event.request;
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

    beforeSendTransaction() {
      // Block all transactions (performance traces).
      return null;
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
 */
export function syncAnalyticsConsent(enabled: boolean): void {
  const client = Sentry.getClient();
  if (!client) return;
  if (!enabled) {
    void Sentry.flush(2000);
  }
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
