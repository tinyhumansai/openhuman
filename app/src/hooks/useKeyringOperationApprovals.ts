import { useCallback, useEffect, useState } from 'react';

import { socketService } from '../services/socketService';

const EVENT_COLON = 'keyring:access_approval_requested';
const EVENT_UNDERSCORE = 'keyring_access_approval_requested';

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

  const handleRequest = useCallback((payload: unknown) => {
    const request = parseRequest(payload);
    if (!request) return;
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
    return () => {
      socketService.off(EVENT_COLON, handleRequest);
      socketService.off(EVENT_UNDERSCORE, handleRequest);
      window.clearInterval(expiryTimer);
    };
  }, [handleRequest]);

  const dismiss = useCallback((requestId: string) => {
    setRequests(current => current.filter(request => request.request_id !== requestId));
  }, []);

  return { requests, dismiss };
}
