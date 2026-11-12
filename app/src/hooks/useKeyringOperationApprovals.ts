import { useCallback, useEffect, useRef, useState } from 'react';

import { listPendingKeyringOperations } from '../services/keyringApi';
import { socketService } from '../services/socketService';

const EVENT_COLON = 'keyring:access_approval_requested';
const EVENT_UNDERSCORE = 'keyring_access_approval_requested';
/** How often to ask the core for pending requests while no socket is connected. */
const POLL_INTERVAL_MS = 1_000;

export type KeyringOperationAccess = 'read' | 'write' | 'delete';

export interface KeyringOperationApprovalRequest {
  request_id: string;
  caller_agent: string;
  origin: string;
  purpose: string;
  access: KeyringOperationAccess;
  name: string;
  expires_at_ms: number;
}

function parseRequest(value: unknown): KeyringOperationApprovalRequest | null {
  if (!value || typeof value !== 'object') return null;
  const payload = value as Record<string, unknown>;
  if (typeof payload.request_id !== 'string' || !payload.request_id) return null;
  if (typeof payload.caller_agent !== 'string' || !payload.caller_agent) return null;
  if (typeof payload.origin !== 'string' || !payload.origin) return null;
  if (typeof payload.purpose !== 'string' || !payload.purpose) return null;
  if (payload.access !== 'read' && payload.access !== 'write' && payload.access !== 'delete') {
    return null;
  }
  if (typeof payload.name !== 'string' || !payload.name) return null;
  if (typeof payload.expires_at_ms !== 'number' || !Number.isFinite(payload.expires_at_ms)) {
    return null;
  }
  if (payload.expires_at_ms <= Date.now()) return null;

  return {
    request_id: payload.request_id,
    caller_agent: payload.caller_agent,
    origin: payload.origin,
    purpose: payload.purpose,
    access: payload.access,
    name: payload.name,
    expires_at_ms: payload.expires_at_ms,
  };
}

/** Listen for exact-operation keyring approval requests, de-duplicated across aliases. */
export function useKeyringOperationApprovals(): {
  requests: KeyringOperationApprovalRequest[];
  dismiss: (requestId: string) => void;
} {
  const [requests, setRequests] = useState<KeyringOperationApprovalRequest[]>([]);
  // A poll that was in flight when the user answered can still list the
  // answered request; never show a dismissed request again.
  const dismissed = useRef(new Set<string>());

  const handleRequest = useCallback((payload: unknown) => {
    const request = parseRequest(payload);
    if (!request || dismissed.current.has(request.request_id)) return;
    setRequests(current =>
      current.some(existing => existing.request_id === request.request_id)
        ? current
        : [...current, request]
    );
  }, []);

  useEffect(() => {
    socketService.on(EVENT_COLON, handleRequest);
    socketService.on(EVENT_UNDERSCORE, handleRequest);
    const expiryTimer = window.setInterval(() => {
      const now = Date.now();
      setRequests(current => current.filter(request => request.expires_at_ms > now));
    }, 1_000);
    // The socket connects only once a session exists, so a keyring write made
    // while signing in would never be shown. Until it connects, ask the core.
    let polling = false;
    const pollTimer = window.setInterval(() => {
      if (polling || socketService.isConnected()) return;
      polling = true;
      listPendingKeyringOperations()
        .then(pending => pending.forEach(handleRequest))
        // A failed poll is retried on the next tick.
        .catch(() => undefined)
        .finally(() => {
          polling = false;
        });
    }, POLL_INTERVAL_MS);
    return () => {
      socketService.off(EVENT_COLON, handleRequest);
      socketService.off(EVENT_UNDERSCORE, handleRequest);
      window.clearInterval(expiryTimer);
      window.clearInterval(pollTimer);
    };
  }, [handleRequest]);

  const dismiss = useCallback((requestId: string) => {
    dismissed.current.add(requestId);
    setRequests(current => current.filter(request => request.request_id !== requestId));
  }, []);

  return { requests, dismiss };
}
