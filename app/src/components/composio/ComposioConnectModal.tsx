/**
 * Modal for connecting / managing a Composio toolkit.
 *
 * Mirrors the flow, positioning, and portal/backdrop plumbing of
 * `SkillSetupModal` so the two feel identical to the user:
 *
 *   disconnected → collect provider-specific required fields (if any) →
 *   "Connect" button → POST composio_authorize → open connectUrl via
 *   tauri-opener → poll listConnections until the toolkit flips to
 *   ACTIVE → "Connected" success screen with a "Disconnect" action.
 *
 * Provider-specific required fields (Jira subdomain, WhatsApp WABA id,
 * Dynamics 365 org name, …) are declared in the
 * [`toolkitRequiredFields`] registry rather than hard-coded as per-toolkit
 * booleans here (#2127). If Composio still returns
 * `ConnectedAccount_MissingRequiredFields` (error code 612) for any toolkit
 * — e.g. a new required field landed backend-side before the registry was
 * updated — the modal transitions to a `needs-fields` recovery phase that
 * collects the same registry fields and retries, instead of surfacing the
 * raw backend error.
 *
 * Redundant refetches from the polling hook in `useComposioIntegrations`
 * keep the Skills page badge in sync too, so the card reflects the new
 * state as soon as the modal closes.
 */
import { type ChangeEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import {
  authorize,
  deleteConnection,
  getUserScopes,
  listConnections,
  setUserScopes,
} from '../../lib/composio/composioApi';
import {
  isMetaOAuthToolkit,
  isOAuthRateLimitedError,
  metaOAuthRateLimitMessage,
} from '../../lib/composio/oauthHandoff';
import {
  type ComposioConnection,
  type ComposioUserScopePref,
  deriveComposioState,
} from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';
import { openUrl } from '../../utils/openUrl';
import type { ComposioToolkitMeta } from './toolkitMeta';
import {
  getRequiredFieldsForToolkit,
  type ToolkitRequiredField,
  validateRequiredFieldValues,
} from './toolkitRequiredFields';
import TriggerToggles from './TriggerToggles';

function deriveConnectionLabel(c: ComposioConnection): string | null {
  for (const value of [c.accountEmail, c.workspace, c.username]) {
    const normalized = value?.trim();
    if (normalized) return normalized;
  }
  return null;
}

/**
 * The Composio error slug for missing required fields (code 612). Matching
 * on the slug string is more precise than matching the numeric code, which
 * could appear in unrelated messages (e.g. port numbers, resource IDs).
 */
const COMPOSIO_MISSING_REQUIRED_FIELDS_SLUG = 'ConnectedAccount_MissingRequiredFields';

/**
 * Validate an Atlassian subdomain. Accepts the short form used in
 * `<subdomain>.atlassian.net` — alphanumerics and hyphens, 1-63 chars,
 * no leading/trailing hyphens. Rejects full URLs so users are not confused
 * about what to paste.
 *
 * Retained for backwards compatibility with consumers that imported the
 * helper directly. The registry in `toolkitRequiredFields.ts` uses the
 * same regex via `validateSubdomainLabel`, shared with Dynamics 365.
 */
export function isValidAtlassianSubdomain(value: string): boolean {
  return /^[a-z0-9][a-z0-9-]{0,61}[a-z0-9]$|^[a-z0-9]$/i.test(value.trim());
}

/**
 * Detect a `ConnectedAccount_MissingRequiredFields` (code 612) error from
 * the backend/Composio. Returns true if the thrown error message contains
 * the known slug. Matching only on the slug avoids false positives from
 * unrelated messages that happen to contain the numeric code "612".
 * Safe to call with any value — returns false for null/non-Error.
 */
export function isMissingRequiredFieldsError(err: unknown): boolean {
  if (!err) return false;
  const msg = err instanceof Error ? err.message : String(err);
  return msg.includes(COMPOSIO_MISSING_REQUIRED_FIELDS_SLUG);
}

/**
 * Return a safe, user-facing summary of an authorization failure. Strips the
 * raw backend URL and JSON payload from the message so sensitive Composio
 * internals are never shown in the UI.
 */
export function sanitizeAuthError(err: unknown): string {
  if (isMissingRequiredFieldsError(err)) {
    // Never surface raw 612 payloads — callers should handle this separately.
    return 'A required field is missing. Please provide the missing details and try again.';
  }
  if (!err) return 'Something went wrong.';
  const raw = err instanceof Error ? err.message : String(err);

  // Strip any URL that looks like a backend endpoint so it is not displayed.
  const stripped = raw.replace(/https?:\/\/[^\s"]+/g, '<backend>');

  // Trim at the first occurrence of a JSON blob to avoid leaking payloads.
  // The URL stripping above may consume the `:` before `{`, so we match
  // the optional colon and any surrounding whitespace before the `{`.
  // This covers both `: {"error"...}` and the bare ` {"error"...}` form.
  const jsonIdx = stripped.search(/\s*:?\s*\{"error"/);
  // Fall back to trimming at any bare `{` that follows whitespace if we
  // did not find a `{"error"` form (defensive — handles other JSON shapes).
  const jsonIdxFallback = stripped.search(/\s\{/);
  const cutIdx =
    jsonIdx !== -1 ? jsonIdx : jsonIdxFallback !== -1 ? jsonIdxFallback : stripped.length;
  const trimmed = stripped.slice(0, cutIdx).trimEnd();

  // Collapse repeated colons / prefixes produced by the RPC error chain.
  // Apply iteratively until stable to handle nested wrapping.
  let result = trimmed;
  let prev: string;
  do {
    prev = result;
    result = result
      .replace(/^(Authorization failed:\s*)+/i, '')
      .replace(/^\[composio\]\s*authorize failed:\s*/i, '')
      .replace(/^Backend returned \d+[^:]*(?:for POST <backend>[^:]*)?:?\s*/i, '')
      .replace(/^Composio authorization failed:\s*/i, '')
      .trim();
  } while (result !== prev);

  return result || 'Authorization failed.';
}

type Phase =
  | 'idle'
  // Recovery phase entered when Composio returns
  // `ConnectedAccount_MissingRequiredFields` (code 612) — the user is asked
  // for the same registry fields again so they can retry.
  | 'needs-fields'
  | 'authorizing'
  | 'waiting'
  | 'connected'
  | 'expired'
  | 'disconnecting'
  | 'error';

interface ComposioConnectModalProps {
  toolkit: ComposioToolkitMeta;
  /** All existing connections for this toolkit (if any) from the hook. */
  connections?: ComposioConnection[];
  /** Connected, but not yet exposed to the agent tool surface. */
  agentUnsupported?: boolean;
  /** Invoked on successful connect/disconnect so the parent can refresh. */
  onChanged?: () => void;
  onClose: () => void;
}

const POLL_INTERVAL_MS = 4_000;
const POLL_TIMEOUT_MS = 5 * 60 * 1_000;

export default function ComposioConnectModal({
  toolkit,
  connections,
  agentUnsupported = false,
  onChanged,
  onClose,
}: ComposioConnectModalProps) {
  const { t } = useT();
  const modalRef = useRef<HTMLDivElement>(null);
  const pollTimerRef = useRef<number | null>(null);
  const pollDeadlineRef = useRef<number>(0);
  const isPollingRef = useRef<boolean>(false);
  const inFlightRef = useRef<boolean>(false);
  const connectInFlightRef = useRef<boolean>(false);
  const [connectInFlight, setConnectInFlight] = useState(false);

  const connection = connections?.[0];
  const initialState = deriveComposioState(connection);
  const initiallyConnected = initialState === 'connected';
  const initiallyExpired = initialState === 'expired';
  const [phase, setPhase] = useState<Phase>(
    initiallyConnected
      ? 'connected'
      : initiallyExpired
        ? 'expired'
        : initialState === 'pending'
          ? 'waiting'
          : 'idle'
  );
  const [error, setError] = useState<string | null>(null);
  const [connectUrl, setConnectUrl] = useState<string | null>(null);
  const [clearMemoryOnDisconnect, setClearMemoryOnDisconnect] = useState(false);

  // Provider-specific required fields are sourced from the declarative
  // registry rather than per-toolkit booleans (#2127). New providers
  // (Dynamics 365 `org_name`, future toolkits, …) only need a registry
  // entry — no per-toolkit branches inside this component.
  const requiredFields = useMemo(() => getRequiredFieldsForToolkit(toolkit.slug), [toolkit.slug]);
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const [activeConnections, setActiveConnections] = useState<ComposioConnection[]>(
    connections?.filter(c => deriveComposioState(c) === 'connected') ?? []
  );
  const [activeConnection, setActiveConnection] = useState<ComposioConnection | undefined>(
    connection
  );

  // ── Scope preferences (read/write/admin) ────────────────────────
  // The pref gates which curated Composio actions the agent may call.
  // We load it lazily once the toolkit is connected, so the toggles in
  // the success view always reflect what the core actually has stored.
  const [scopes, setScopes] = useState<ComposioUserScopePref | null>(null);
  const [scopeError, setScopeError] = useState<string | null>(null);
  // Per-key in-flight flag so spamming a single toggle disables only
  // that row while the RPC round-trips.
  const [savingScope, setSavingScope] = useState<keyof ComposioUserScopePref | null>(null);

  // Escape to close
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [onClose]);

  // Focus trap
  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    modalRef.current?.focus();
    return () => {
      previousFocus?.focus?.();
    };
  }, []);

  const stopPolling = useCallback(() => {
    isPollingRef.current = false;
    if (pollTimerRef.current != null) {
      window.clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  }, []);

  // Cleanup on unmount
  useEffect(() => () => stopPolling(), [stopPolling]);

  const startPolling = useCallback(() => {
    stopPolling();
    isPollingRef.current = true;
    pollDeadlineRef.current = Date.now() + POLL_TIMEOUT_MS;

    const scheduleNext = () => {
      if (!isPollingRef.current) return;
      pollTimerRef.current = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
    };

    const tick = async () => {
      // Guard against overlapping executions: if a previous tick is still
      // in flight or we've already stopped/deadlined, skip this round.
      if (inFlightRef.current || !isPollingRef.current) return;
      if (Date.now() > pollDeadlineRef.current) {
        stopPolling();
        setPhase('error');
        setError(t('composio.connect.oauthTimeout'));
        return;
      }
      inFlightRef.current = true;
      try {
        const resp = await listConnections();
        const allForToolkit = resp.connections.filter(
          c => c.toolkit.toLowerCase() === toolkit.slug.toLowerCase()
        );
        const hit =
          allForToolkit.find(
            c => deriveComposioState(c) !== 'connected' && deriveComposioState(c) !== 'disconnected'
          ) ?? allForToolkit[0];
        if (hit) {
          setActiveConnection(hit);
          setActiveConnections(allForToolkit.filter(c => deriveComposioState(c) === 'connected'));
          const state = deriveComposioState(hit);
          if (state === 'connected') {
            stopPolling();
            setPhase('connected');
            setError(null);
            onChanged?.();
            return;
          }
          if (state === 'error') {
            stopPolling();
            setPhase('error');
            setError(
              t('composio.connect.connectionFailed').replace('{status}', String(hit.status))
            );
            return;
          }
          if (state === 'expired') {
            stopPolling();
            setPhase('expired');
            setError(null);
            return;
          }
        }
      } catch (err) {
        // Swallow transient errors during polling — we'll retry on next tick.
        console.warn('[composio] poll failed:', err);
      } finally {
        inFlightRef.current = false;
      }
      scheduleNext();
    };

    // Fire once immediately, then recurse via setTimeout once the previous
    // tick resolves. Avoids overlapping async ticks entirely.
    void tick();
  }, [onChanged, stopPolling, t, toolkit.slug]);

  // If the modal opens while an OAuth handoff is already in flight
  // (status = PENDING/INITIATED/…), resume polling instead of asking
  // the user to click Connect again.
  useEffect(() => {
    if (initialState === 'pending') {
      startPolling();
    }
    // intentionally run once on mount — startPolling has stable deps and
    // re-running this on every identity change would restart the poller.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /**
   * Validate registry-declared required fields. Populates `fieldErrors`
   * with per-field i18n keys when any are missing or malformed, and
   * returns true only when every field is valid.
   */
  const validateRequiredFields = useCallback((): boolean => {
    if (requiredFields.length === 0) return true;
    const errors = validateRequiredFieldValues(requiredFields, fieldValues);
    setFieldErrors(errors);
    return Object.keys(errors).length === 0;
  }, [requiredFields, fieldValues]);

  const handleConnect = useCallback(async () => {
    if (connectInFlightRef.current) {
      console.debug(
        '[composio][authorize] ignored duplicate Connect click toolkit=%s',
        toolkit.slug
      );
      return;
    }
    if (!validateRequiredFields()) return;

    connectInFlightRef.current = true;
    setConnectInFlight(true);
    setPhase('authorizing');
    setError(null);
    setFieldErrors({});
    setConnectUrl(null);

    const extraParams: Record<string, string> = {};
    for (const field of requiredFields) {
      const value = (fieldValues[field.key] ?? '').trim();
      if (value) extraParams[field.key] = value;
    }

    console.debug(
      '[composio][authorize] → toolkit=%s has_extra_params=%s field_count=%d',
      toolkit.slug,
      Object.keys(extraParams).length > 0,
      requiredFields.length
    );

    try {
      const resp = await authorize(
        toolkit.slug,
        Object.keys(extraParams).length > 0 ? extraParams : undefined
      );
      console.debug(
        '[composio][authorize] ← toolkit=%s connection_id=%s',
        toolkit.slug,
        resp.connectionId
      );
      setConnectUrl(resp.connectUrl);
      setPhase('waiting');
      startPolling();
      try {
        await openUrl(resp.connectUrl);
      } catch (openErr) {
        console.warn('[composio][authorize] failed to open connectUrl:', openErr);
      }
    } catch (err) {
      console.error(
        '[composio][authorize] failed toolkit=%s slug_check=%s',
        toolkit.slug,
        isMissingRequiredFieldsError(err)
      );

      if (isMissingRequiredFieldsError(err)) {
        // Composio reported a missing required field (code 612). When the
        // registry has any required-field entries for this toolkit, drop
        // into the `needs-fields` recovery phase so the user can supply the
        // missing value and retry inline. When the registry does not yet
        // know about the missing field — e.g. Composio backend just added a
        // new required field — fall back to a sanitized error message so
        // the user is not stuck on a recovery form that cannot succeed.
        console.debug(
          '[composio][authorize] missing-required-fields toolkit=%s registry_field_count=%d',
          toolkit.slug,
          requiredFields.length
        );
        if (requiredFields.length > 0) {
          setPhase('needs-fields');
          setError(null);
        } else {
          setPhase('error');
          setError(t('composio.connect.additionalConfigRequired'));
        }
        return;
      }

      setPhase('error');
      if (isMetaOAuthToolkit(toolkit.slug) && isOAuthRateLimitedError(err)) {
        setError(metaOAuthRateLimitMessage(toolkit.name));
      } else {
        setError(sanitizeAuthError(err));
      }
    } finally {
      connectInFlightRef.current = false;
      setConnectInFlight(false);
    }
  }, [
    validateRequiredFields,
    requiredFields,
    fieldValues,
    startPolling,
    toolkit.slug,
    toolkit.name,
    t,
  ]);

  // Fetch the stored scope pref whenever the modal lands in the
  // 'connected' phase. Re-fetching each time we transition (rather
  // than once on mount) keeps the toggles correct after a fresh OAuth
  // handoff completes inside this modal.
  useEffect(() => {
    if (phase !== 'connected') return;
    let cancelled = false;
    void (async () => {
      try {
        const pref = await getUserScopes(toolkit.slug);
        if (!cancelled) setScopes(pref);
      } catch (err) {
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          setScopeError(`${t('composio.connect.scopeLoadError')}: ${msg}`);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [phase, t, toolkit.slug]);

  const handleToggleScope = useCallback(
    async (key: keyof ComposioUserScopePref) => {
      if (!scopes || savingScope) {
        console.debug(
          '[composio][scopes] toggle ignored toolkit=%s key=%s reason=%s',
          toolkit.slug,
          key,
          !scopes ? 'pref-not-loaded' : 'another-save-in-flight'
        );
        return;
      }
      const optimistic: ComposioUserScopePref = { ...scopes, [key]: !scopes[key] };
      console.debug(
        '[composio][scopes] toggle toolkit=%s key=%s old=%s new=%s',
        toolkit.slug,
        key,
        scopes[key],
        optimistic[key]
      );
      setScopes(optimistic);
      setSavingScope(key);
      setScopeError(null);
      try {
        const persisted = await setUserScopes(toolkit.slug, optimistic);
        console.debug(
          '[composio][scopes] toggle persisted toolkit=%s key=%s pref=%o',
          toolkit.slug,
          key,
          persisted
        );
        setScopes(persisted);
      } catch (err) {
        // Roll back on failure so the toggle reflects reality.
        const msg = err instanceof Error ? err.message : String(err);
        console.error(
          '[composio][scopes] toggle failed toolkit=%s key=%s error=%o',
          toolkit.slug,
          key,
          err
        );
        setScopes(scopes);
        setScopeError(`${t('composio.connect.scopeSaveError').replace('{key}', key)}: ${msg}`);
      } finally {
        setSavingScope(null);
      }
    },
    [savingScope, scopes, t, toolkit.slug]
  );

  const handleDisconnect = useCallback(
    async (targetConnection?: ComposioConnection) => {
      const conn = targetConnection ?? activeConnection;
      if (!conn) return;
      setPhase('disconnecting');
      setError(null);
      try {
        await deleteConnection(conn.id, { clearMemory: clearMemoryOnDisconnect });
        const remaining = activeConnections.filter(c => c.id !== conn.id);
        setActiveConnections(remaining);
        if (remaining.length > 0) {
          setActiveConnection(remaining[0]);
          setClearMemoryOnDisconnect(false);
          setPhase('connected');
        } else {
          setActiveConnection(undefined);
          setClearMemoryOnDisconnect(false);
          setPhase('idle');
        }
        onChanged?.();
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setPhase('error');
        setError(t('composio.connect.disconnectFailed').replace('{msg}', msg));
        setClearMemoryOnDisconnect(false);
      }
    },
    [activeConnection, activeConnections, clearMemoryOnDisconnect, onChanged, t]
  );

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const headerTitle =
    phase === 'connected'
      ? `${t('composio.connect.manage')} ${toolkit.name}`
      : phase === 'expired'
        ? `${t('composio.reconnect')} ${toolkit.name}`
        : `${t('composio.connect.connect')} ${toolkit.name}`;

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="composio-setup-title">
      <div
        ref={modalRef}
        className="bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 rounded-3xl shadow-large w-full max-w-[460px] overflow-hidden animate-fade-up focus:outline-none focus:ring-0"
        style={{
          animationDuration: '200ms',
          animationTimingFunction: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
          animationFillMode: 'both',
        }}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-4 border-b border-stone-200 dark:border-neutral-800">
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0 pr-2">
              <div className="flex items-center gap-2">
                {toolkit.icon}
                <h2
                  id="composio-setup-title"
                  className="text-base font-semibold text-stone-900 dark:text-neutral-100">
                  {headerTitle}
                </h2>
              </div>
              <p className="text-xs text-stone-400 dark:text-neutral-500 mt-1.5 line-clamp-2">
                {toolkit.description}
              </p>
            </div>
            <button
              type="button"
              onClick={onClose}
              className="p-1 text-stone-400 dark:text-neutral-500 hover:text-stone-900 dark:hover:text-neutral-100 dark:text-neutral-100 dark:hover:text-neutral-100 transition-colors rounded-lg hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 flex-shrink-0"
              aria-label={t('common.close')}>
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          </div>
        </div>

        {/* Body */}
        <div className="p-4 space-y-3">
          {phase === 'idle' && (
            <>
              <p className="text-sm text-stone-600 dark:text-neutral-300">
                {`${t('composio.connect.idleDescription')} ${toolkit.name} ${t('composio.connect.idleDescriptionSuffix')}`}
              </p>
              <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
                <p className="mt-1 text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
                  {toolkit.name} {t('composio.connect.permissionsNote')}{' '}
                  <span className="font-medium">{toolkit.permissionLabel}</span>.{' '}
                  {t('composio.connect.permissionsNoteSuffix')}
                </p>
              </div>
              <RequiredFieldsForm
                fields={requiredFields}
                values={fieldValues}
                errors={fieldErrors}
                onChange={(key, v) => {
                  setFieldValues(prev => ({ ...prev, [key]: v }));
                  if (fieldErrors[key]) {
                    setFieldErrors(prev => {
                      const next = { ...prev };
                      delete next[key];
                      return next;
                    });
                  }
                }}
              />
              {error && phase === 'idle' && <p className="text-[11px] text-coral-600">{error}</p>}
              <button
                type="button"
                disabled={connectInFlight}
                onClick={() => void handleConnect()}
                className="w-full rounded-xl bg-primary-500 text-white text-sm font-medium py-2.5 hover:bg-primary-600 transition-colors disabled:opacity-60 disabled:cursor-not-allowed">
                {`${t('composio.connect.connect')} ${toolkit.name}`}
              </button>
            </>
          )}

          {phase === 'needs-fields' && (
            <>
              <p className="text-sm text-stone-600 dark:text-neutral-300">
                {`${t('composio.connect.needsFieldsPrefix')} ${toolkit.name} ${t('composio.connect.needsFieldsSuffix')}`}
              </p>
              <RequiredFieldsForm
                fields={requiredFields}
                values={fieldValues}
                errors={fieldErrors}
                autoFocusFirst
                onChange={(key, v) => {
                  setFieldValues(prev => ({ ...prev, [key]: v }));
                  if (fieldErrors[key]) {
                    setFieldErrors(prev => {
                      const next = { ...prev };
                      delete next[key];
                      return next;
                    });
                  }
                }}
              />
              <button
                type="button"
                disabled={connectInFlight}
                onClick={() => void handleConnect()}
                className="w-full rounded-xl bg-primary-500 text-white text-sm font-medium py-2.5 hover:bg-primary-600 transition-colors disabled:opacity-60 disabled:cursor-not-allowed">
                {t('composio.connect.retryConnection')}
              </button>
              <button
                type="button"
                onClick={() => {
                  setPhase('idle');
                  setFieldErrors({});
                  setError(null);
                }}
                className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-600 dark:text-neutral-300 text-xs font-medium py-2 hover:bg-stone-50 dark:hover:bg-neutral-800/60 transition-colors">
                {t('common.cancel')}
              </button>
            </>
          )}

          {phase === 'authorizing' && (
            <p className="text-sm text-stone-500 dark:text-neutral-400">
              {t('composio.connect.requestingUrl')}
            </p>
          )}

          {phase === 'waiting' && (
            <>
              <div className="flex items-center gap-2 text-sm text-stone-700 dark:text-neutral-200">
                <div className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
                {`${t('composio.connect.waitingFor')} ${toolkit.name} ${t('composio.connect.oauthComplete')}`}
              </div>
              {connectUrl && (
                <button
                  type="button"
                  onClick={() => void openUrl(connectUrl)}
                  className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 text-stone-700 dark:text-neutral-200 text-xs font-medium py-2 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 transition-colors">
                  {t('composio.connect.reopenBrowser')}
                </button>
              )}
              <p className="text-xs text-stone-400 dark:text-neutral-500">
                {t('composio.connect.waitingHint')}
              </p>
            </>
          )}

          {phase === 'expired' && (
            <>
              <div className="rounded-xl border border-coral-200 bg-coral-50 p-3">
                <div className="flex items-center gap-2 text-sm font-medium text-coral-800">
                  <div className="w-2 h-2 rounded-full bg-coral-500" />
                  {t('composio.expiredAuthorization').replace('{name}', toolkit.name)}
                </div>
                <p className="mt-2 text-xs leading-relaxed text-coral-700">
                  {t('composio.expiredDescription').replace('{name}', toolkit.name)}
                </p>
              </div>
              <button
                type="button"
                disabled={connectInFlight}
                onClick={() => void handleConnect()}
                className="w-full rounded-xl bg-primary-500 text-white text-sm font-medium py-2.5 hover:bg-primary-600 transition-colors disabled:opacity-60 disabled:cursor-not-allowed">
                {`${t('composio.reconnect')} ${toolkit.name}`}
              </button>
            </>
          )}

          {phase === 'connected' && (
            <>
              {/* Single connection: inline status (backward-compatible view) */}
              {activeConnections.length <= 1 && (
                <div className="flex items-center gap-2 text-sm text-sage-700">
                  <div className="w-2 h-2 rounded-full bg-sage-500" />
                  <div>
                    {`${toolkit.name} ${t('composio.connect.isConnected')}`} &nbsp;
                    {(activeConnections[0] ?? activeConnection) &&
                      deriveConnectionLabel(activeConnections[0] ?? activeConnection!) && (
                        <span className="text-[11px] text-stone-400 dark:text-neutral-500 font-mono">
                          ({deriveConnectionLabel((activeConnections[0] ?? activeConnection)!)})
                        </span>
                      )}
                  </div>
                </div>
              )}
              {/* Multiple connections: list with per-connection controls */}
              {activeConnections.length > 1 && (
                <div className="space-y-2">
                  <p className="text-xs font-medium text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
                    {t('composio.connect.connectedAccounts')} ({activeConnections.length})
                  </p>
                  {activeConnections.map(conn => (
                    <div
                      key={conn.id}
                      className="flex items-center justify-between gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
                      <div className="flex items-center gap-2 min-w-0">
                        <div className="w-2 h-2 rounded-full bg-sage-500 shrink-0" />
                        <span className="text-sm text-stone-800 dark:text-neutral-100 truncate">
                          {deriveConnectionLabel(conn) ?? toolkit.name}
                        </span>
                        {conn.id === activeConnections[0]?.id && (
                          <span className="text-[10px] font-medium text-primary-600 dark:text-primary-400 bg-primary-50 dark:bg-primary-500/10 px-1.5 py-0.5 rounded-full shrink-0">
                            {t('composio.connect.defaultLabel')}
                          </span>
                        )}
                      </div>
                      <button
                        type="button"
                        onClick={() => void handleDisconnect(conn)}
                        className="text-[11px] text-coral-600 hover:text-coral-700 font-medium shrink-0">
                        {t('composio.connect.disconnectAccount')}
                      </button>
                    </div>
                  ))}
                </div>
              )}
              {agentUnsupported && (
                <div className="rounded-xl border border-amber-200 bg-amber-50 p-3 dark:border-amber-500/30 dark:bg-amber-500/10">
                  <div className="flex items-center gap-2 text-sm font-medium text-amber-800 dark:text-amber-200">
                    <div className="h-2 w-2 rounded-full bg-amber-500" />
                    {t('composio.previewBadge')}
                  </div>
                  <p className="mt-2 text-xs leading-relaxed text-amber-700 dark:text-amber-200/80">
                    {t('composio.previewTooltip')}
                  </p>
                </div>
              )}
              <ScopeToggles
                scopes={scopes}
                savingScope={savingScope}
                onToggle={handleToggleScope}
                error={scopeError}
              />
              {activeConnection && (
                <TriggerToggles
                  toolkitSlug={toolkit.slug}
                  toolkitName={toolkit.name}
                  connectionId={activeConnection.id}
                />
              )}
              <button
                type="button"
                disabled={connectInFlight}
                onClick={() => void handleConnect()}
                className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-700 dark:text-neutral-200 text-sm font-medium py-2.5 hover:bg-stone-50 dark:hover:bg-neutral-800/60 transition-colors disabled:opacity-60 disabled:cursor-not-allowed">
                {t('composio.connect.addAnotherAccount')}
              </button>
              <label className="flex items-start gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
                <input
                  type="checkbox"
                  checked={clearMemoryOnDisconnect}
                  onChange={event => setClearMemoryOnDisconnect(event.currentTarget.checked)}
                  className="mt-0.5 h-4 w-4 rounded border-stone-300 text-primary-600 focus:ring-primary-500"
                />
                <span className="min-w-0">
                  <span className="block text-sm font-medium text-stone-800 dark:text-neutral-100">
                    {t('accounts.disconnectClearMemory')}
                  </span>
                  <span className="block text-xs text-stone-500 dark:text-neutral-400">
                    {t('accounts.disconnectClearMemoryHint')}
                  </span>
                </span>
              </label>
              <div className="grid grid-cols-2 gap-3">
                <button
                  type="button"
                  onClick={() => void handleDisconnect()}
                  className="w-full rounded-xl border border-coral-200 bg-coral-50 text-coral-700 text-sm font-medium py-2.5 hover:bg-coral-100 transition-colors">
                  {t('skills.disconnect')}
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl bg-primary-500 text-white text-sm font-medium py-2.5 hover:bg-primary-600 transition-colors">
                  {t('common.close')}
                </button>
              </div>
            </>
          )}

          {phase === 'disconnecting' && (
            <p className="text-sm text-stone-500 dark:text-neutral-400">
              {t('composio.connect.disconnecting')}
            </p>
          )}

          {phase === 'error' && (
            <>
              <div className="rounded-xl border border-coral-200 bg-coral-50 p-3">
                <p className="text-sm text-coral-700">{error ?? t('misc.somethingWentWrong')}</p>
              </div>
              <button
                type="button"
                onClick={() => {
                  setClearMemoryOnDisconnect(false);
                  setPhase(
                    initiallyConnected ? 'connected' : initiallyExpired ? 'expired' : 'idle'
                  );
                  setError(null);
                }}
                className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-700 dark:text-neutral-200 text-sm font-medium py-2 hover:bg-stone-50 dark:hover:bg-neutral-800/60 transition-colors">
                {t('common.dismiss')}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );

  return createPortal(modalContent, document.body);
}

// ── Scope toggles ───────────────────────────────────────────────────

type ScopeRowDef = { key: keyof ComposioUserScopePref; labelKey: string; hintKey: string };

const SCOPE_ROWS: Array<ScopeRowDef> = [
  {
    key: 'read',
    labelKey: 'composio.connect.scope.read',
    hintKey: 'composio.connect.scope.readHint',
  },
  {
    key: 'write',
    labelKey: 'composio.connect.scope.write',
    hintKey: 'composio.connect.scope.writeHint',
  },
  {
    key: 'admin',
    labelKey: 'composio.connect.scope.admin',
    hintKey: 'composio.connect.scope.adminHint',
  },
];

interface ScopeTogglesProps {
  scopes: ComposioUserScopePref | null;
  savingScope: keyof ComposioUserScopePref | null;
  onToggle: (key: keyof ComposioUserScopePref) => void;
  error: string | null;
}

function ScopeToggles({ scopes, savingScope, onToggle, error }: ScopeTogglesProps) {
  const { t } = useT();
  // Render skeleton placeholders while we wait on the initial load so
  // the modal layout doesn't jump when the pref arrives.
  const loading = scopes === null;

  return (
    <div className="border-t border-stone-100 dark:border-neutral-800 pt-3 mt-1 space-y-2">
      <div className="flex items-baseline justify-between">
        <h3 className="text-xs font-semibold text-stone-700 dark:text-neutral-200 uppercase tracking-wide">
          {t('composio.connect.permissions')}
        </h3>
        <p className="text-[10px] text-stone-400 dark:text-neutral-500">
          {t('composio.connect.permissionsDefault')}
        </p>
      </div>
      <ul className="space-y-1.5">
        {SCOPE_ROWS.map(row => {
          const enabled = scopes?.[row.key] ?? false;
          const isSaving = savingScope === row.key;
          const rowLabel = t(row.labelKey as Parameters<typeof t>[0]);
          const rowHint = t(row.hintKey as Parameters<typeof t>[0]);
          return (
            <li
              key={row.key}
              className="flex items-start justify-between gap-3 rounded-lg px-2 py-1.5 hover:bg-stone-50 dark:hover:bg-neutral-800/60">
              <div className="min-w-0 flex-1">
                <span className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                  {rowLabel}
                </span>
                <p className="text-[11px] text-stone-400 dark:text-neutral-500 leading-snug">
                  {rowHint}
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={enabled}
                aria-label={`${enabled ? t('common.disable') : t('common.enable')} ${rowLabel} scope`}
                disabled={loading || savingScope !== null}
                onClick={() => onToggle(row.key)}
                className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50 ${
                  enabled ? 'bg-primary-500' : 'bg-stone-300'
                }`}>
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${
                    enabled ? 'translate-x-5' : 'translate-x-0.5'
                  } ${isSaving ? 'animate-pulse' : ''}`}
                />
              </button>
            </li>
          );
        })}
      </ul>
      {error && <p className="text-[11px] text-coral-600">{error}</p>}
    </div>
  );
}

// ── Generic required-fields form ────────────────────────────────────

interface RequiredFieldsFormProps {
  fields: readonly ToolkitRequiredField[];
  values: Record<string, string>;
  errors: Record<string, string>;
  onChange: (key: string, value: string) => void;
  /** Autofocus the first input on mount (used by the `needs-fields` recovery phase). */
  autoFocusFirst?: boolean;
}

/**
 * Generic renderer for provider-specific required fields declared in
 * `toolkitRequiredFields.ts`. Replaces the per-toolkit
 * `AtlassianSubdomainInput` / `WabaIdInput` blocks (#2127). Each field
 * shows a label, optional fixed suffix inside the input
 * (e.g. `.atlassian.net`), an optional hint, and an inline error message
 * driven by the `errors` map (keyed by field key, value is an i18n key).
 */
function RequiredFieldsForm({
  fields,
  values,
  errors,
  onChange,
  autoFocusFirst,
}: RequiredFieldsFormProps) {
  const { t } = useT();
  if (fields.length === 0) return null;
  return (
    <>
      {fields.map((field, idx) => {
        const inputId = `composio-required-${field.key}`;
        const hintId = `${inputId}-hint`;
        const value = values[field.key] ?? '';
        const errorKey = errors[field.key];
        const errorText = errorKey ? t(errorKey) : null;
        return (
          <div key={field.key} className="space-y-1.5">
            <label
              htmlFor={inputId}
              className="block text-xs font-medium text-stone-700 dark:text-neutral-200">
              {t(field.labelKey)}
              <span className="ml-1 text-coral-500">*</span>
            </label>
            <div className="flex items-center rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 focus-within:border-primary-400 focus-within:ring-2 focus-within:ring-primary-100 overflow-hidden">
              <input
                id={inputId}
                data-testid={inputId}
                type="text"
                value={value}
                autoFocus={autoFocusFirst && idx === 0}
                onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(field.key, e.target.value)}
                placeholder={field.placeholder}
                aria-describedby={hintId}
                aria-invalid={!!errorText}
                className="flex-1 min-w-0 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 bg-transparent focus:outline-none"
              />
              {field.suffix && (
                <span className="pr-3 text-xs text-stone-400 dark:text-neutral-500 select-none whitespace-nowrap">
                  {field.suffix}
                </span>
              )}
            </div>
            {/* Always render the hint paragraph with the same id so
                aria-describedby resolves regardless of error state. */}
            {errorText ? (
              <p id={hintId} role="alert" className="text-[11px] text-coral-600">
                {errorText}
              </p>
            ) : (
              field.hintKey && (
                <p
                  id={hintId}
                  className="text-[11px] leading-relaxed text-stone-400 dark:text-neutral-500">
                  {t(field.hintKey)}
                </p>
              )
            )}
          </div>
        );
      })}
    </>
  );
}
