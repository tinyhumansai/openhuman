import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

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
import { isMissingRequiredFieldsError, sanitizeAuthError } from './composioAuthErrors';
import type { ComposioToolkitMeta } from './toolkitMeta';
import { getRequiredFieldsForToolkit, validateRequiredFieldValues } from './toolkitRequiredFields';

export type ComposioConnectPhase =
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

interface UseComposioConnectFlowArgs {
  toolkit: ComposioToolkitMeta;
  /** All existing connections for this toolkit (if any) from the hook. */
  connections?: ComposioConnection[];
  /** Invoked on successful connect/disconnect so the parent can refresh. */
  onChanged?: () => void;
}

// Confirmation is purely poll-based: Composio has no deep-link callback and,
// in direct mode, its v3 link endpoint returns no stable connection id, so we
// must re-list connections and match by toolkit. To make the "Connected" flip
// feel fast without hammering the backend, the cadence starts short and backs
// off toward a cap, and a window focus / tab-visible event (the user switching
// back from the browser after authorizing — a near-perfect "just finished"
// signal) pokes an immediate re-poll and resets the cadence to fast.
const POLL_INTERVAL_START_MS = 1_500;
const POLL_INTERVAL_MAX_MS = 4_000;
const POLL_BACKOFF_FACTOR = 1.5;
const POLL_TIMEOUT_MS = 5 * 60 * 1_000;

/**
 * Owns the entire connect/poll/disconnect/scope state machine for
 * `ComposioConnectModal`, split out purely to keep that file under the
 * repo's ~500-line budget — no behavior changes from the inline version.
 */
export function useComposioConnectFlow({
  toolkit,
  connections,
  onChanged,
}: UseComposioConnectFlowArgs) {
  const { t } = useT();
  const pollTimerRef = useRef<number | null>(null);
  const pollDeadlineRef = useRef<number>(0);
  const pollIntervalRef = useRef<number>(POLL_INTERVAL_START_MS);
  const isPollingRef = useRef<boolean>(false);
  const inFlightRef = useRef<boolean>(false);
  // Set while polling to fire an immediate re-poll (e.g. on window focus).
  const pokePollRef = useRef<() => void>(() => {});
  const connectInFlightRef = useRef<boolean>(false);
  const [connectInFlight, setConnectInFlight] = useState(false);

  const connection = connections?.[0];
  const initialState = deriveComposioState(connection);
  const initiallyConnected = initialState === 'connected';
  const initiallyExpired = initialState === 'expired';
  const [phase, setPhase] = useState<ComposioConnectPhase>(
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

  const stopPolling = useCallback(() => {
    isPollingRef.current = false;
    pokePollRef.current = () => {};
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
    pollIntervalRef.current = POLL_INTERVAL_START_MS;

    const scheduleNext = () => {
      if (!isPollingRef.current) return;
      pollTimerRef.current = window.setTimeout(() => void tick(), pollIntervalRef.current);
      // Back off toward the cap so a long wait doesn't hammer the backend.
      pollIntervalRef.current = Math.min(
        POLL_INTERVAL_MAX_MS,
        Math.round(pollIntervalRef.current * POLL_BACKOFF_FACTOR)
      );
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

    // Poke an immediate re-poll (used when the window regains focus). Cancels
    // the pending scheduled tick, resets the cadence to fast so the next few
    // rounds are quick, and fires now. The in-flight guard inside `tick`
    // prevents overlap if a round is already running.
    pokePollRef.current = () => {
      if (!isPollingRef.current || inFlightRef.current) return;
      if (Date.now() > pollDeadlineRef.current) return;
      if (pollTimerRef.current != null) {
        window.clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
      pollIntervalRef.current = POLL_INTERVAL_START_MS;
      void tick();
    };

    // Fire once immediately, then recurse via setTimeout once the previous
    // tick resolves. Avoids overlapping async ticks entirely.
    void tick();
  }, [onChanged, stopPolling, t, toolkit.slug]);

  // When the user returns to the app after authorizing in the browser, the
  // window regains focus / the tab becomes visible — poll immediately instead
  // of waiting for the next scheduled tick, so the "Connected" flip feels
  // instant. No-op unless a poll is currently active.
  useEffect(() => {
    const poke = () => pokePollRef.current();
    const onVisibility = () => {
      if (document.visibilityState === 'visible') poke();
    };
    window.addEventListener('focus', poke);
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      window.removeEventListener('focus', poke);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, []);

  // If the modal opens while an OAuth handoff is already in flight
  // (status = PENDING/INITIATED/…), resume polling instead of asking
  // the user to click Connect again.
  useEffect(() => {
    if (initialState === 'pending') {
      startPolling();
    }
    // intentionally run once on mount — startPolling has stable deps and
    // re-running this on every identity change would restart the poller.
    //
    // No `eslint-disable` for `react-hooks/exhaustive-deps` here: the plugin is
    // registered only for `**/*.jsx` / `**/*.tsx` (eslint.config.js), so in this
    // `.ts` file the directive named an undefined rule, which is an error in its
    // own right. Hook rules simply do not run on this file today.
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

  return {
    t,
    phase,
    setPhase,
    error,
    setError,
    connectUrl,
    clearMemoryOnDisconnect,
    setClearMemoryOnDisconnect,
    requiredFields,
    fieldValues,
    setFieldValues,
    fieldErrors,
    setFieldErrors,
    activeConnections,
    activeConnection,
    scopes,
    scopeError,
    savingScope,
    connectInFlight,
    initiallyConnected,
    initiallyExpired,
    handleConnect,
    handleToggleScope,
    handleDisconnect,
  };
}
