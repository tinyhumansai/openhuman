/**
 * Analytics & Sentry service
 *
 * Initializes Sentry for error reporting and OpenPanel for privacy-limited
 * usage tracking. Both are gated on user analytics consent.
 *
 * Sentry privacy guarantees enforced in `beforeSend`:
 *   - No breadcrumbs, requests, extras, or arbitrary contexts (only OS /
 *     browser / device metadata kept)
 *   - No frame-level locals or source-context snippets
 *   - No PII — `user` is reduced to a stable account id (or omitted)
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
  APP_BINARY_VERSION,
  APP_ENVIRONMENT,
  APP_VERSION,
  BUILD_SHA,
  CORE_CARGO_VERSION,
  GA_MEASUREMENT_ID,
  IS_DEV,
  OPENPANEL_API_URL,
  OPENPANEL_CLIENT_ID,
  SENTRY_DSN,
  SENTRY_RELEASE,
  SENTRY_SMOKE_TEST,
  SUPPORT_URL,
  TAURI_CARGO_VERSION,
} from '../utils/config';
import { CoreRpcError } from './coreRpcClient';

// ---------------------------------------------------------------------------
// Google Analytics 4 typings — raw gtag.js API
// ---------------------------------------------------------------------------

type GtagCommand = 'config' | 'event' | 'set' | 'js';
interface GtagFn {
  (...args: [GtagCommand, ...unknown[]]): void;
}

declare global {
  interface Window {
    dataLayer: unknown[];
    gtag: GtagFn;
  }
}

const OPENPANEL_TRACK_URL = `${OPENPANEL_API_URL}/track`;
const MAX_PENDING_ANALYTICS_EVENTS = 20;
const INTERACTIVE_CLICK_SELECTOR =
  'button,a,[role="button"],summary,[data-track],[data-analytics-id],[data-testid],[data-walkthrough]';
const CONTROL_CHANGE_SELECTOR =
  'select,input[type="checkbox"],input[type="radio"],input[type="range"],[role="switch"],[role="checkbox"],[role="radio"]';

// ---------------------------------------------------------------------------
// Module-level state
// ---------------------------------------------------------------------------

let gaInitialized = false;
let opInitialized = false;
let analyticsConsentSynced = false;
let uiInteractionTrackingStarted = false;

type AnalyticsParams = Record<string, string | number | boolean>;

interface PendingAnalyticsEvent {
  type: 'event' | 'page_view';
  name: string;
  params?: AnalyticsParams;
}

const pendingAnalyticsEvents: PendingAnalyticsEvent[] = [];

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
  'tab_bar_change',
  'tauri_browser_click',
  'ui_click',
  'ui_control_change',
  'ui_form_submit',
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
      // #3963: `defaultIntegrations: false` (above) drops the integration that
      // consumes the top-level `ignoreErrors` option (below), so the intended
      // noise filter has been dead config since it was added. Re-include it
      // explicitly so `ignoreErrors` runs again. It executes as an event
      // processor *before* `beforeSend`, so the consent/privacy logic there is
      // unaffected — this only restores the pre-`beforeSend` drop of the four
      // benign `ResizeObserver loop` / network-noise patterns.
      Sentry.inboundFiltersIntegration(),
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

      // Seed a support deep link keyed on this event's own id so the crash
      // report links back to the support channel (#3980). Set as a TAG, not
      // an `extra` — the privacy scrub above deletes `event.extra`, but the
      // event id is known here and tags survive. The URL carries only the
      // event's own id + a static base (no PII). Mirrors the id the user
      // sees + copies on `ErrorFallbackScreen`.
      if (event.event_id) {
        const sep = SUPPORT_URL.includes('?') ? '&' : '?';
        event.tags.support_url = `${SUPPORT_URL}${sep}ref=${event.event_id}`;
      }

      // Strip PII; keep a stable account id only.
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
  analyticsConsentSynced = true;
  if (gaInitialized || opInitialized) {
    console.debug(`[analytics] consent updated: enabled=${enabled}`);
  }
  if (enabled) {
    initializeAnalyticsProviders();
    flushPendingAnalyticsEvents();
  } else {
    pendingAnalyticsEvents.length = 0;
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
  if (opInitialized || !OPENPANEL_CLIENT_ID || !OPENPANEL_API_URL) return;
  opInitialized = true;
  console.debug('[analytics] OpenPanel initialized (direct ingestion)', {
    clientId: OPENPANEL_CLIENT_ID,
    apiUrl: OPENPANEL_API_URL,
  });
}

function initializeAnalyticsProviders(): void {
  initGoogleAnalytics();
  initOpenPanel();
}

/**
 * Initialize all analytics providers (GA4 + OpenPanel).
 * Idempotent — each provider initializes at most once.
 */
export function initGA(): void {
  analyticsEnabled = isAnalyticsEnabled();
  if (analyticsEnabled) {
    initializeAnalyticsProviders();
    flushPendingAnalyticsEvents();
  }
}

/**
 * Send a privacy-limited page view to all initialized providers.
 */
export function trackPageView(path: string): void {
  const pagePath = normalizeAnalyticsPagePath(path);
  if (!analyticsEnabled) {
    queuePendingAnalyticsEvent({
      type: 'page_view',
      name: 'screen_view',
      params: { page: pagePath },
    });
    return;
  }
  if (!gaInitialized && !opInitialized) return;
  console.debug('[analytics] trackPageView', { path: pagePath });
  const properties = { page: pagePath, __path: pagePath, ...analyticsPageContextProperties() };
  if (gaInitialized) {
    window.gtag('event', 'page_view', {
      page_path: pagePath,
      page_location: analyticsPageLocation(),
      ...properties,
    });
  }
  if (opInitialized) {
    void sendOpenPanelTrack('screen_view', { ...properties, __title: currentDocumentTitle() });
  }
}

/**
 * Send a privacy-limited feature-engagement event to all initialized providers.
 *
 * Event names must appear in `ALLOWED_EVENTS`. Calls with unlisted names
 * are dropped and a console warning is emitted.
 */
export function trackEvent(eventName: string, params?: AnalyticsParams): void {
  if (!ALLOWED_EVENTS.has(eventName)) {
    console.warn(
      `[analytics] trackEvent dropped — '${eventName}' is not in ALLOWED_EVENTS allowlist`
    );
    return;
  }

  if (!analyticsEnabled) {
    queuePendingAnalyticsEvent({ type: 'event', name: eventName, params });
    return;
  }
  if (!gaInitialized && !opInitialized) return;

  const properties = { ...(params ?? {}), ...analyticsContextProperties() };
  const loggableProperties = { ...properties };
  delete loggableProperties.user_id;
  console.debug('[analytics] trackEvent', { eventName, params: loggableProperties });
  if (gaInitialized) window.gtag('event', eventName, properties);
  if (opInitialized) {
    void sendOpenPanelTrack(eventName, properties);
  }
}

export function startUiInteractionTracking(): () => void {
  if (uiInteractionTrackingStarted || typeof document === 'undefined') return () => undefined;
  uiInteractionTrackingStarted = true;

  const handleClick = (event: MouseEvent) => {
    const target = event.target instanceof Element ? event.target : null;
    const element = target?.closest(INTERACTIVE_CLICK_SELECTOR);
    if (!(element instanceof HTMLElement) || shouldSkipInteractionElement(element)) return;

    trackEvent('ui_click', {
      ...interactionBaseProperties(element),
      interaction_kind: 'click',
      control_id: controlIdentifier(element),
      destination: destinationForElement(element),
    });
  };

  const handleChange = (event: Event) => {
    const target = event.target instanceof Element ? event.target : null;
    const element = target?.closest(CONTROL_CHANGE_SELECTOR);
    if (!(element instanceof HTMLElement) || shouldSkipInteractionElement(element)) return;

    trackEvent('ui_control_change', {
      ...interactionBaseProperties(element),
      interaction_kind: 'change',
      control_id: controlIdentifier(element),
      control_state: controlState(element),
    });
  };

  const handleSubmit = (event: SubmitEvent) => {
    const form = event.target instanceof HTMLFormElement ? event.target : null;
    if (!form || shouldSkipInteractionElement(form)) return;

    trackEvent('ui_form_submit', {
      ...interactionBaseProperties(form),
      interaction_kind: 'submit',
      control_id: controlIdentifier(form),
    });
  };

  document.addEventListener('click', handleClick, true);
  document.addEventListener('change', handleChange, true);
  document.addEventListener('submit', handleSubmit, true);

  return () => {
    document.removeEventListener('click', handleClick, true);
    document.removeEventListener('change', handleChange, true);
    document.removeEventListener('submit', handleSubmit, true);
    uiInteractionTrackingStarted = false;
  };
}

function queuePendingAnalyticsEvent(event: PendingAnalyticsEvent): void {
  if (analyticsConsentSynced) return;
  pendingAnalyticsEvents.push(event);
  if (pendingAnalyticsEvents.length > MAX_PENDING_ANALYTICS_EVENTS) {
    pendingAnalyticsEvents.splice(0, pendingAnalyticsEvents.length - MAX_PENDING_ANALYTICS_EVENTS);
  }
}

function flushPendingAnalyticsEvents(): void {
  if (!analyticsEnabled || pendingAnalyticsEvents.length === 0) return;
  const events = pendingAnalyticsEvents.splice(0, pendingAnalyticsEvents.length);
  for (const event of events) {
    if (event.type === 'page_view') {
      trackPageView(String(event.params?.page ?? event.name));
    } else {
      trackEvent(event.name, event.params);
    }
  }
}

function interactionBaseProperties(element: HTMLElement): AnalyticsParams {
  return {
    page: currentAppPath(),
    page_hash: currentPageHash(),
    element_tag: element.tagName.toLowerCase(),
    element_role: scrubIdentifier(element.getAttribute('role')) ?? '',
    element_type: scrubIdentifier(element.getAttribute('type')) ?? '',
  };
}

function controlIdentifier(element: HTMLElement): string {
  const explicit =
    element.getAttribute('data-analytics-id') ??
    element.getAttribute('data-track') ??
    element.getAttribute('data-testid') ??
    element.getAttribute('data-walkthrough') ??
    element.getAttribute('name') ??
    element.id;
  const scrubbed = scrubIdentifier(explicit);
  if (scrubbed) return scrubbed;

  const hrefDestination = destinationForElement(element);
  if (hrefDestination) return `link_${scrubIdentifier(hrefDestination) ?? 'internal'}`;

  const container = nearestStableContainer(element);
  const tag = element.tagName.toLowerCase();
  if (container) return `${tag}_in_${container}`;
  return tag;
}

function destinationForElement(element: HTMLElement): string {
  const href = element instanceof HTMLAnchorElement ? element.getAttribute('href') : null;
  if (!href) return '';
  if (href.startsWith('#/')) return href.slice(1);
  if (href.startsWith('/')) return href;
  return href.startsWith('http') ? 'external' : '';
}

function controlState(element: HTMLElement): string {
  if (element instanceof HTMLInputElement) {
    if (element.type === 'checkbox' || element.type === 'radio') {
      return element.checked ? 'checked' : 'unchecked';
    }
    if (element.type === 'range') return 'changed';
  }
  if (element instanceof HTMLSelectElement) return 'selected';

  const ariaChecked = element.getAttribute('aria-checked');
  if (ariaChecked === 'true' || ariaChecked === 'false' || ariaChecked === 'mixed') {
    return ariaChecked;
  }
  return 'changed';
}

function nearestStableContainer(element: HTMLElement): string | undefined {
  const container = element.closest('[data-testid],[data-walkthrough],[data-analytics-id]');
  if (!(container instanceof HTMLElement) || container === element) return undefined;
  return scrubIdentifier(
    container.getAttribute('data-analytics-id') ??
      container.getAttribute('data-testid') ??
      container.getAttribute('data-walkthrough')
  );
}

function shouldSkipInteractionElement(element: HTMLElement): boolean {
  if (element.closest('[data-analytics-skip="true"],[data-no-analytics="true"]')) return true;
  if (element.closest('[contenteditable="true"]')) return true;
  if (element instanceof HTMLInputElement) {
    return ['text', 'search', 'email', 'password', 'tel', 'url', 'number', 'file'].includes(
      element.type
    );
  }
  if (element instanceof HTMLTextAreaElement) return true;
  return false;
}

function scrubIdentifier(value: string | null | undefined): string | undefined {
  const trimmed = value?.trim();
  if (!trimmed) return undefined;
  const withoutQuery = trimmed.split(/[?#]/)[0] ?? trimmed;
  const scrubbed = withoutQuery
    .replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, ':email')
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, ':id')
    .replace(/\b[0-9a-f]{16,}\b/gi, ':id')
    .replace(/\b\d{3,}\b/g, ':num')
    .replace(/[^a-zA-Z0-9:_/-]+/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_+|_+$/g, '')
    .toLowerCase()
    .slice(0, 80);
  return scrubbed || undefined;
}

function currentAppPath(): string {
  if (typeof window === 'undefined') return '';
  return normalizeAnalyticsPagePath(window.location.pathname);
}

function currentPageHash(): string {
  if (typeof window === 'undefined') return '';
  return window.location.hash.startsWith('#/') ? window.location.hash : '';
}

function normalizeAnalyticsPagePath(path: string): string {
  if (typeof window !== 'undefined' && window.location.hash.startsWith('#/')) {
    return hashToPath(window.location.hash);
  }
  if (path.startsWith('#/')) return hashToPath(path);
  return path || '/';
}

function hashToPath(hash: string): string {
  const withoutHash = hash.slice(1);
  return withoutHash || '/';
}

async function sendOpenPanelTrack(eventName: string, params?: AnalyticsParams): Promise<void> {
  const profileId = currentAnalyticsUserId();
  const properties = {
    __path: currentOpenPanelPath(),
    __referrer: currentDocumentReferrer(),
    __timestamp: new Date().toISOString(),
    ...(params ?? {}),
  };

  try {
    const response = await fetch(OPENPANEL_TRACK_URL, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'openpanel-client-id': OPENPANEL_CLIENT_ID,
        'openpanel-sdk-name': 'openhuman-react',
        'openpanel-sdk-version': '0.1.0',
      },
      body: JSON.stringify({
        type: 'track',
        payload: { name: eventName, ...(profileId ? { profileId } : {}), properties },
      }),
      keepalive: true,
    });

    if (!response.ok) {
      console.warn('[analytics] OpenPanel track failed:', response.status, await response.text());
    }
  } catch (err) {
    console.warn('[analytics] OpenPanel track failed:', err);
  }
}

function analyticsContextProperties(): AnalyticsParams {
  const userId = currentAnalyticsUserId();
  return {
    user_id: userId ?? '',
    app_version: APP_VERSION,
    binary_version: APP_BINARY_VERSION,
    core_cargo_version: CORE_CARGO_VERSION,
    tauri_cargo_version: TAURI_CARGO_VERSION,
    release: SENTRY_RELEASE,
    build_sha: BUILD_SHA,
    app_environment: APP_ENVIRONMENT,
  };
}

function analyticsPageContextProperties(): AnalyticsParams {
  return { ...analyticsContextProperties(), page_hash: currentPageHash() };
}

function currentAnalyticsUserId(): string | undefined {
  return getCoreStateSnapshot().snapshot.currentUser?._id;
}

function currentOpenPanelPath(): string {
  return currentAppPath();
}

function analyticsPageLocation(): string {
  if (typeof window === 'undefined') return '';
  const pagePath = currentAppPath();
  return `${window.location.origin}${pagePath}`;
}

function currentDocumentTitle(): string {
  if (typeof document === 'undefined') return '';
  return document.title;
}

function currentDocumentReferrer(): string {
  if (typeof document === 'undefined') return '';
  return document.referrer;
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
