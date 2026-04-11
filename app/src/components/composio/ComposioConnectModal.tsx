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
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { authorize, deleteConnection, listConnections } from '../../lib/composio/composioApi';
import { type ComposioConnection, deriveComposioState } from '../../lib/composio/types';
import { openUrl } from '../../utils/openUrl';
import type { ComposioToolkitMeta } from './toolkitMeta';

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
  const [activeConnection, setActiveConnection] = useState<ComposioConnection | undefined>(
    connection
  );

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
    setPhase('authorizing');
    setError(null);
    setConnectUrl(null);
    try {
      const resp = await authorize(toolkit.slug);
      setConnectUrl(resp.connectUrl);
      await openUrl(resp.connectUrl);
      setPhase('waiting');
      startPolling();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setPhase('error');
      setError(`Authorization failed: ${msg}`);
    }
  }, [startPolling, toolkit.slug]);

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
                <span className="text-lg">{toolkit.icon}</span>
                <h2 id="composio-setup-title" className="text-base font-semibold text-stone-900">
                  {headerTitle}
                </h2>
                <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-md bg-primary-500/15 text-primary-600">
                  composio
                </span>
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
                Connect your {toolkit.name} account through Composio. We will open a browser window
                where you can grant access, and then this app will detect the connection
                automatically.
              </p>
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
                {toolkit.name} is connected.
              </div>
              {activeConnection && (
                <p className="text-[11px] text-stone-400 font-mono break-all">
                  id: {activeConnection.id}
                </p>
              )}
              <button
                type="button"
                onClick={() => void handleDisconnect()}
                className="w-full rounded-xl border border-coral-200 bg-coral-50 text-coral-700 text-sm font-medium py-2.5 hover:bg-coral-100 transition-colors">
                Disconnect
              </button>
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
