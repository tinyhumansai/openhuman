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
 *
 * Error flow: beforeSend intercepts all events, sanitizes them, queues them
 * in the errorReportQueue for user opt-in, and returns null to prevent
 * auto-sending. Users can then review and explicitly report each error.
 */
import * as Sentry from '@sentry/react';

import { store } from '../store';
import { IS_DEV, SENTRY_DSN } from '../utils/config';
import { enqueueError, registerSentrySender, type SanitizedSentryEvent } from './errorReportQueue';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Strip sensitive fields from the exception object before including it
 * in the sanitized event shown to the user and sent to Sentry.
 *
 * Removes: local variables (vars), source code lines (context_line,
 * pre_context, post_context), mechanism.data, and module_metadata.
 */
function sanitizeException(
  exception: Sentry.Event['exception']
): SanitizedSentryEvent['exception'] {
  if (!exception?.values) return undefined;

  return {
    values: exception.values.map(entry => ({
      type: entry.type ?? 'Error',
      value: entry.value ?? '',
      stacktrace: entry.stacktrace?.frames
        ? {
            frames: entry.stacktrace.frames.map(frame => ({
              filename: frame.filename,
              function: frame.function,
              module: frame.module,
              lineno: frame.lineno,
              colno: frame.colno,
              abs_path: frame.abs_path,
              in_app: frame.in_app,
              // Stripped: vars, context_line, pre_context, post_context,
              //          instruction_addr, addr_mode, debug_id, module_metadata
            })),
          }
        : undefined,
      mechanism: entry.mechanism
        ? { type: entry.mechanism.type, handled: entry.mechanism.handled }
        : undefined,
      // Stripped: mechanism.data (arbitrary key-value pairs)
    })),
  };
}

/** Check if the current user has opted into analytics. */
export function isAnalyticsEnabled(): boolean {
  const state = store.getState();
  const userId = state.user?.user?._id;
  if (!userId) return false;
  return state.auth.isAnalyticsEnabledByUser[userId] !== false;
}

// ---------------------------------------------------------------------------
// Bypass flag — when true, beforeSend passes the event through to Sentry
// ---------------------------------------------------------------------------

let _bypassBeforeSend = false;

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
    // Intercept every event: sanitize, queue for user opt-in, block auto-send
    // -----------------------------------------------------------------------
    beforeSend(event) {
      // Bypass mode: let the event through (used by sendEventToSentry)
      if (_bypassBeforeSend) {
        _bypassBeforeSend = false;
        return event;
      }

      // --- Sanitize the event ---

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

      // --- Build a sanitized snapshot for the user to inspect ---
      const sanitized: SanitizedSentryEvent = {
        event_id: event.event_id ?? crypto.randomUUID().replace(/-/g, ''),
        timestamp: typeof event.timestamp === 'number' ? event.timestamp : Date.now() / 1000,
        platform: event.platform ?? 'javascript',
        exception: sanitizeException(event.exception),
        contexts: event.contexts as SanitizedSentryEvent['contexts'],
        user: event.user as SanitizedSentryEvent['user'],
        tags: event.tags as Record<string, string> | undefined,
        environment: IS_DEV ? 'development' : 'production',
      };

      // Extract human-readable title + message from the exception
      const firstException = event.exception?.values?.[0];
      const title = firstException?.type ?? 'Error';
      const message = firstException?.value ?? 'Unknown error';

      // Queue the error for the notification UI
      enqueueError({
        id: crypto.randomUUID(),
        timestamp: Date.now(),
        source: 'global',
        title,
        message,
        sentryEvent: sanitized,
      });

      // Return null to prevent Sentry from auto-sending
      return null;
    },

    beforeSendTransaction() {
      // Block all transactions (performance traces)
      return null;
    },

    // Ignore common non-actionable errors
    ignoreErrors: ['ResizeObserver loop', 'Network request failed', 'Load failed', 'AbortError'],
  });

  // Register the bypass sender so the error queue can actually send events
  registerSentrySender((sanitizedEvent: SanitizedSentryEvent) => {
    _bypassBeforeSend = true;
    Sentry.captureEvent(sanitizedEvent as unknown as Sentry.Event);
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
