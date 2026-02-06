/**
 * Analytics & Sentry service
 *
 * Manages Sentry error reporting gated behind user analytics consent.
 * Designed to capture ONLY:
 *   - Error message & stack trace
 *   - Device / browser metadata (via Sentry's default user-agent parsing)
 *   - Source file location (via source maps)
 *
 * Explicitly strips:
 *   - All breadcrumbs (console, click, network, etc.)
 *   - Redux state / localStorage / sessionStorage
 *   - User PII (IP address, cookies)
 *   - Request bodies / headers
 *   - Session replay
 */
import * as Sentry from '@sentry/react';

import { store } from '../store';

const SENTRY_DSN = import.meta.env.VITE_SENTRY_DSN as string | undefined;
const IS_DEV = Boolean(import.meta.env.DEV) || import.meta.env.MODE === 'development';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Check if the current user has opted into analytics. */
export function isAnalyticsEnabled(): boolean {
  const state = store.getState();
  const userId = state.user?.user?._id;
  if (!userId) return false;
  return state.auth.isAnalyticsEnabledByUser[userId] !== false;
}

// ---------------------------------------------------------------------------
// Sentry initialisation
// ---------------------------------------------------------------------------

export function initSentry(): void {
  if (!SENTRY_DSN) return;

  Sentry.init({
    dsn: SENTRY_DSN,
    environment: IS_DEV ? 'development' : 'production',
    enabled: !IS_DEV, // disable in dev builds

    // -----------------------------------------------------------------------
    // Privacy: disable EVERYTHING that could leak sensitive state
    // -----------------------------------------------------------------------

    // No session replay
    replaysSessionSampleRate: 0,
    replaysOnErrorSampleRate: 0,

    // No performance / tracing
    tracesSampleRate: 0,

    // No breadcrumbs at all (console, clicks, network, etc.)
    defaultIntegrations: false,
    integrations: [
      // Only keep the bare-minimum integrations for stack traces
      Sentry.functionToStringIntegration(),
      Sentry.linkedErrorsIntegration(),
      Sentry.dedupeIntegration(),
      Sentry.browserApiErrorsIntegration(),
      Sentry.globalHandlersIntegration(),
    ],

    // Strip IP address
    sendDefaultPii: false,

    // -----------------------------------------------------------------------
    // Gate every event behind the user's analytics consent flag
    // -----------------------------------------------------------------------
    beforeSend(event) {
      if (!isAnalyticsEnabled()) return null;

      // Strip any breadcrumbs that somehow snuck in
      event.breadcrumbs = [];

      // Strip request data (cookies, headers, body)
      delete event.request;

      // Strip user PII — keep only a stable anonymous ID
      const userId = store.getState().user?.user?._id;
      event.user = userId ? { id: userId } : undefined;

      // Strip any extra/contexts that could contain Redux or localStorage data
      delete event.extra;
      event.contexts = {
        // Keep only OS / browser / device metadata
        os: event.contexts?.os,
        browser: event.contexts?.browser,
        device: event.contexts?.device,
      };

      return event;
    },

    beforeSendTransaction() {
      // Block all transactions (performance traces)
      return null;
    },

    // Ignore common non-actionable errors
    ignoreErrors: ['ResizeObserver loop', 'Network request failed', 'Load failed', 'AbortError'],
  });
}

// ---------------------------------------------------------------------------
// Consent sync — call when the user toggles analytics on/off
// ---------------------------------------------------------------------------

/**
 * Re-sync Sentry's enabled state after the user changes their consent.
 * Called from onboarding and settings.
 */
export function syncAnalyticsConsent(enabled: boolean): void {
  const client = Sentry.getClient();
  if (!client) return;

  if (enabled) {
    // Client is already initialised; events will pass through beforeSend
    // because isAnalyticsEnabled() will now return true.
  } else {
    // Flush any pending events, then future events will be dropped by beforeSend.
    void Sentry.flush(2000);
  }
}
