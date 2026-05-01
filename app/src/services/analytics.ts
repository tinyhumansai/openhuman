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
import { APP_ENVIRONMENT, IS_DEV, SENTRY_DSN, SENTRY_RELEASE } from '../utils/config';

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
      // Drop events when the user hasn't opted into analytics.
      if (!isAnalyticsEnabled()) return null;

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
  // message before returning. No-op when unset.
  if (import.meta.env.VITE_SENTRY_SMOKE_TEST === 'true') {
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
