/**
 * Modal for connecting / managing a Composio toolkit.
 *
 * Mirrors the flow, positioning, and portal/backdrop plumbing of
 * `SkillSetupModal` so the two feel identical to the user:
 *
 *   disconnected → "Connect" button → POST composio_authorize →
 *   open connectUrl via tauri-opener → poll listConnections until
 *   the toolkit flips to ACTIVE → "Connected" success screen with
 *   a "Disconnect" action.
 *
 * Redundant refetches from the polling hook in `useComposioIntegrations`
 * keep the Skills page badge in sync too, so the card reflects the new
 * state as soon as the modal closes.
 */
import { type ChangeEvent, useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import {
  authorize,
  deleteConnection,
  getUserScopes,
  listConnections,
  setUserScopes,
} from '../../lib/composio/composioApi';
import {
  type ComposioConnection,
  type ComposioUserScopePref,
  deriveComposioState,
} from '../../lib/composio/types';
import { openUrl } from '../../utils/openUrl';
import type { ComposioToolkitMeta } from './toolkitMeta';
import TriggerToggles from './TriggerToggles';

function deriveConnectionLabel(c: ComposioConnection): string | null {
  for (const value of [c.accountEmail, c.workspace, c.username]) {
    const normalized = value?.trim();
    if (normalized) return normalized;
  }
  return null;
}

type Phase = 'idle' | 'authorizing' | 'waiting' | 'connected' | 'disconnecting' | 'error';

interface ComposioConnectModalProps {
  toolkit: ComposioToolkitMeta;
  /** Existing connection (if any) from the hook. */
  connection?: ComposioConnection;
  /** Invoked on successful connect/disconnect so the parent can refresh. */
  onChanged?: () => void;
  onClose: () => void;
}

const POLL_INTERVAL_MS = 4_000;
const POLL_TIMEOUT_MS = 5 * 60 * 1_000;

export default function ComposioConnectModal({
  toolkit,
  connection,
  onChanged,
  onClose,
}: ComposioConnectModalProps) {
  const modalRef = useRef<HTMLDivElement>(null);
  const pollTimerRef = useRef<number | null>(null);
  const pollDeadlineRef = useRef<number>(0);
  const isPollingRef = useRef<boolean>(false);
  const inFlightRef = useRef<boolean>(false);

  const initialState = deriveComposioState(connection);
  const initiallyConnected = initialState === 'connected';
  const [phase, setPhase] = useState<Phase>(
    initiallyConnected ? 'connected' : initialState === 'pending' ? 'waiting' : 'idle'
  );
  const [error, setError] = useState<string | null>(null);
  const [connectUrl, setConnectUrl] = useState<string | null>(null);
  // WhatsApp Business requires a WABA ID before the OAuth flow can start.
  const [wabaId, setWabaId] = useState('');
  const needsWabaId = toolkit.slug === 'whatsapp';
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
        setError(
          'Timed out waiting for OAuth to complete. Please retry or check that the browser finished the flow.'
        );
        return;
      }
      inFlightRef.current = true;
      try {
        const resp = await listConnections();
        const hit = resp.connections.find(
          c => c.toolkit.toLowerCase() === toolkit.slug.toLowerCase()
        );
        if (hit) {
          setActiveConnection(hit);
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
            setError(`Connection failed (status: ${hit.status}).`);
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
  }, [onChanged, stopPolling, toolkit.slug]);

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

  const handleConnect = useCallback(async () => {
    if (needsWabaId && !wabaId.trim()) {
      setError('Please enter your WhatsApp Business Account ID (WABA ID) to continue.');
      return;
    }
    setPhase('authorizing');
    setError(null);
    setConnectUrl(null);
    try {
      const extraParams = needsWabaId ? { waba_id: wabaId.trim() } : undefined;
      const resp = await authorize(toolkit.slug, extraParams);
      setConnectUrl(resp.connectUrl);
      await openUrl(resp.connectUrl);
      setPhase('waiting');
      startPolling();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setPhase('error');
      setError(`Authorization failed: ${msg}`);
    }
  }, [needsWabaId, startPolling, toolkit.slug, wabaId]);

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
          setScopeError(`Couldn't load scope preferences: ${msg}`);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [phase, toolkit.slug]);

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
        setScopeError(`Couldn't save ${key} scope: ${msg}`);
      } finally {
        setSavingScope(null);
      }
    },
    [savingScope, scopes, toolkit.slug]
  );

  const handleDisconnect = useCallback(async () => {
    if (!activeConnection) return;
    setPhase('disconnecting');
    setError(null);
    try {
      await deleteConnection(activeConnection.id);
      setActiveConnection(undefined);
      setPhase('idle');
      onChanged?.();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setPhase('error');
      setError(`Disconnect failed: ${msg}`);
    }
  }, [activeConnection, onChanged]);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const headerTitle = phase === 'connected' ? `Manage ${toolkit.name}` : `Connect ${toolkit.name}`;

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="composio-setup-title">
      <div
        ref={modalRef}
        className="bg-white border border-stone-200 rounded-3xl shadow-large w-full max-w-[460px] overflow-hidden animate-fade-up focus:outline-none focus:ring-0"
        style={{
          animationDuration: '200ms',
          animationTimingFunction: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
          animationFillMode: 'both',
        }}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-4 border-b border-stone-200">
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0 pr-2">
              <div className="flex items-center gap-2">
                {toolkit.icon}
                <h2 id="composio-setup-title" className="text-base font-semibold text-stone-900">
                  {headerTitle}
                </h2>
              </div>
              <p className="text-xs text-stone-400 mt-1.5 line-clamp-2">{toolkit.description}</p>
            </div>
            <button
              type="button"
              onClick={onClose}
              className="p-1 text-stone-400 hover:text-stone-900 transition-colors rounded-lg hover:bg-stone-100 flex-shrink-0"
              aria-label="Close">
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
              <p className="text-sm text-stone-600">
                Connect your {toolkit.name} account. We&apos;ll open a browser window, you approve
                access there, and this app will detect the connection automatically.
              </p>
              <div className="rounded-xl border border-stone-200 bg-stone-50 p-3">
                <p className="mt-1 text-xs leading-relaxed text-stone-600">
                  {toolkit.name} can expose{' '}
                  <span className="font-medium">{toolkit.permissionLabel}</span>. After you connect,
                  OpenHuman&apos;s own agent permissions are controlled below as read, write, and
                  admin toggles.
                </p>
              </div>
              {needsWabaId && (
                <div className="space-y-1.5">
                  <label
                    htmlFor="waba-id-input"
                    className="block text-xs font-medium text-stone-700">
                    WhatsApp Business Account ID (WABA ID)
                    <span className="ml-1 text-coral-500">*</span>
                  </label>
                  <input
                    id="waba-id-input"
                    type="text"
                    value={wabaId}
                    onChange={(e: ChangeEvent<HTMLInputElement>) => {
                      setWabaId(e.target.value);
                      if (error) setError(null);
                    }}
                    placeholder="e.g. 123456789012345"
                    className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-400 focus:outline-none focus:ring-2 focus:ring-primary-100"
                  />
                  <p className="text-[11px] leading-relaxed text-stone-400">
                    Find it via <span className="font-mono">GET /me/businesses</span> then{' '}
                    <span className="font-mono">
                      GET /&#123;business_id&#125;/owned_whatsapp_business_accounts
                    </span>{' '}
                    using your Meta access token.
                  </p>
                </div>
              )}
              {error && phase === 'idle' && <p className="text-[11px] text-coral-600">{error}</p>}
              <button
                type="button"
                onClick={() => void handleConnect()}
                className="w-full rounded-xl bg-primary-500 text-white text-sm font-medium py-2.5 hover:bg-primary-600 transition-colors">
                Connect {toolkit.name}
              </button>
            </>
          )}

          {phase === 'authorizing' && (
            <p className="text-sm text-stone-500">Requesting connect URL…</p>
          )}

          {phase === 'waiting' && (
            <>
              <div className="flex items-center gap-2 text-sm text-stone-700">
                <div className="w-2 h-2 rounded-full bg-amber-500 animate-pulse" />
                Waiting for {toolkit.name} OAuth to complete…
              </div>
              {connectUrl && (
                <button
                  type="button"
                  onClick={() => void openUrl(connectUrl)}
                  className="w-full rounded-xl border border-stone-200 bg-stone-50 text-stone-700 text-xs font-medium py-2 hover:bg-stone-100 transition-colors">
                  Reopen browser
                </button>
              )}
              <p className="text-xs text-stone-400">
                Complete the sign-in in your browser. This window will update when the connection is
                active.
              </p>
            </>
          )}

          {phase === 'connected' && (
            <>
              <div className="flex items-center gap-2 text-sm text-sage-700">
                <div className="w-2 h-2 rounded-full bg-sage-500" />
                <div>
                  {toolkit.name} is connected. &nbsp;
                  {activeConnection && deriveConnectionLabel(activeConnection) && (
                    <span className="text-[11px] text-stone-400 font-mono">
                      ({deriveConnectionLabel(activeConnection)})
                    </span>
                  )}
                </div>
              </div>
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
              <div className="grid grid-cols-2 gap-3">
                <button
                  type="button"
                  onClick={() => void handleDisconnect()}
                  className="w-full rounded-xl border border-coral-200 bg-coral-50 text-coral-700 text-sm font-medium py-2.5 hover:bg-coral-100 transition-colors">
                  Disconnect
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl bg-primary-500 text-white text-sm font-medium py-2.5 hover:bg-primary-600 transition-colors">
                  Close
                </button>
              </div>
            </>
          )}

          {phase === 'disconnecting' && <p className="text-sm text-stone-500">Disconnecting…</p>}

          {phase === 'error' && (
            <>
              <div className="rounded-xl border border-coral-200 bg-coral-50 p-3">
                <p className="text-sm text-coral-700">{error ?? 'Something went wrong.'}</p>
              </div>
              <button
                type="button"
                onClick={() => {
                  setPhase(initiallyConnected ? 'connected' : 'idle');
                  setError(null);
                }}
                className="w-full rounded-xl border border-stone-200 bg-white text-stone-700 text-sm font-medium py-2 hover:bg-stone-50 transition-colors">
                Dismiss
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

const SCOPE_ROWS: Array<{ key: keyof ComposioUserScopePref; label: string; hint: string }> = [
  {
    key: 'read',
    label: 'Read',
    hint: 'Allow listing, fetching, searching (e.g. read emails / pages).',
  },
  {
    key: 'write',
    label: 'Write',
    hint: 'Allow sending, creating, updating (e.g. send emails, create pages).',
  },
  {
    key: 'admin',
    label: 'Admin',
    hint: 'Allow destructive or permission-changing actions (delete, share, etc.).',
  },
];

interface ScopeTogglesProps {
  scopes: ComposioUserScopePref | null;
  savingScope: keyof ComposioUserScopePref | null;
  onToggle: (key: keyof ComposioUserScopePref) => void;
  error: string | null;
}

function ScopeToggles({ scopes, savingScope, onToggle, error }: ScopeTogglesProps) {
  // Render skeleton placeholders while we wait on the initial load so
  // the modal layout doesn't jump when the pref arrives.
  const loading = scopes === null;

  return (
    <div className="border-t border-stone-100 pt-3 mt-1 space-y-2">
      <div className="flex items-baseline justify-between">
        <h3 className="text-xs font-semibold text-stone-700 uppercase tracking-wide">
          Permissions
        </h3>
        <p className="text-[10px] text-stone-400">Read + Write enabled by default</p>
      </div>
      <ul className="space-y-1.5">
        {SCOPE_ROWS.map(row => {
          const enabled = scopes?.[row.key] ?? false;
          const isSaving = savingScope === row.key;
          return (
            <li
              key={row.key}
              className="flex items-start justify-between gap-3 rounded-lg px-2 py-1.5 hover:bg-stone-50">
              <div className="min-w-0 flex-1">
                <span className="text-sm font-medium text-stone-900">{row.label}</span>
                <p className="text-[11px] text-stone-400 leading-snug">{row.hint}</p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={enabled}
                aria-label={`${enabled ? 'Disable' : 'Enable'} ${row.label} scope`}
                disabled={loading || savingScope !== null}
                onClick={() => onToggle(row.key)}
                className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50 ${
                  enabled ? 'bg-primary-500' : 'bg-stone-300'
                }`}>
                <span
                  className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform ${
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
