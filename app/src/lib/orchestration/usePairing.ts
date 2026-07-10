/**
 * Shared pairing data hook for the Orchestration sub-pages.
 *
 * Wraps `apiClient.orchestrationPairing.*` (the tiny.place agent contacts graph)
 * so the Connections, Discover, and Usage panels all read one consistent
 * snapshot and run mutating actions (link / accept / decline / block) through the
 * same load→act→reload lifecycle. Kept separate from `useOrchestrationChats`
 * (which owns the chat transcript surface) so a page can pull in only what it
 * needs.
 */
import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { type PairingSnapshot, PaymentRequiredError } from '../agentworld/invokeApiClient';

const debug = debugFactory('orchestration:pairing');

export type PairingState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'payment_required' }
  | { status: 'ok'; snapshot: PairingSnapshot };

export interface UsePairingResult {
  state: PairingState;
  /** Re-fetch the pairing snapshot. */
  reload: () => Promise<void>;
  /** Run a mutating pairing action, then reload; tracks pending id + error. */
  runAction: (actionId: string, fn: () => Promise<unknown>) => Promise<void>;
  /** The id passed to the currently in-flight `runAction`, or null. */
  pendingAction: string | null;
  /** The last action error message, cleared when a new action starts. */
  actionError: string | null;
}

export function usePairing(): UsePairingResult {
  const [state, setState] = useState<PairingState>({ status: 'loading' });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const reload = useCallback(async () => {
    debug('reload: entry');
    setState(prev => (prev.status === 'ok' ? prev : { status: 'loading' }));
    try {
      const snapshot = await apiClient.orchestrationPairing.list();
      if (!mountedRef.current) return;
      debug(
        'reload: ok contacts=%d incoming=%d outgoing=%d',
        snapshot.contacts.contacts.length,
        snapshot.requests.incoming.length,
        snapshot.requests.outgoing.length
      );
      setState({ status: 'ok', snapshot });
    } catch (error) {
      if (!mountedRef.current) return;
      if (error instanceof PaymentRequiredError) {
        debug('reload: payment_required');
        setState({ status: 'payment_required' });
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      debug('reload: error %s', message);
      setState({ status: 'error', message });
    }
  }, []);

  const runAction = useCallback(
    async (actionId: string, fn: () => Promise<unknown>) => {
      debug('runAction: entry id=%s', actionId);
      setPendingAction(actionId);
      setActionError(null);
      try {
        await fn();
        if (!mountedRef.current) return;
        debug('runAction: success id=%s', actionId);
        await reload();
      } catch (error) {
        if (!mountedRef.current) return;
        const message = error instanceof Error ? error.message : String(error);
        debug('runAction: error id=%s %s', actionId, message);
        setActionError(message);
      } finally {
        if (mountedRef.current) setPendingAction(null);
      }
    },
    [reload]
  );

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void reload(), 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [reload]);

  return { state, reload, runAction, pendingAction, actionError };
}
