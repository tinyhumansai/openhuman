import debug from 'debug';
import { useCallback, useEffect, useSyncExternalStore } from 'react';

import { fetchPendingApprovals, type PendingApproval } from '../services/api/approvalApi';

const log = debug('flows:pending-approvals-store');
const POLL_INTERVAL_MS = 2000;

export interface FlowPendingApprovalsSnapshot {
  approvals: PendingApproval[];
  error: string | null;
  polling: boolean;
}

function cloneAndFreeze<T>(value: T): T {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) {
    return Object.freeze(value.map(item => cloneAndFreeze(item))) as T;
  }
  return Object.freeze(
    Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, item]) => [
        key,
        cloneAndFreeze(item),
      ])
    )
  ) as T;
}

const freezeApprovals = (approvals: PendingApproval[]): PendingApproval[] =>
  cloneAndFreeze(approvals);

const makeSnapshot = (
  approvals: PendingApproval[],
  error: string | null,
  polling: boolean
): FlowPendingApprovalsSnapshot =>
  Object.freeze({ approvals: freezeApprovals(approvals), error, polling });

const INITIAL_SNAPSHOT = makeSnapshot([], null, false);

let snapshot = INITIAL_SNAPSHOT;
let retainCount = 0;
let pollTimer: number | undefined;
let requestGeneration = 0;
let nextRequestId = 0;
let activeRequestId: number | null = null;
let inFlight: Promise<void> | null = null;
let queuedRefresh: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(next: FlowPendingApprovalsSnapshot): void {
  snapshot = next;
  for (const listener of listeners) listener();
}

function normalizeError(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === 'string' && error.trim()) return error;
  return 'Unable to load pending approvals';
}

function clearPollTimer(): void {
  if (pollTimer === undefined) return;
  window.clearTimeout(pollTimer);
  pollTimer = undefined;
}

function scheduleNextPoll(generation: number): void {
  if (retainCount === 0 || generation !== requestGeneration || pollTimer !== undefined) return;
  pollTimer = window.setTimeout(() => {
    pollTimer = undefined;
    void startRefresh(true);
  }, POLL_INTERVAL_MS);
}

export function subscribeFlowPendingApprovals(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getFlowPendingApprovalsSnapshot(): FlowPendingApprovalsSnapshot {
  return snapshot;
}

function startRefresh(publishOnlyWhenRetained = false): Promise<void> {
  clearPollTimer();
  const generation = requestGeneration;
  const requestId = ++nextRequestId;
  activeRequestId = requestId;
  const request = (async () => {
    try {
      const approvals = await fetchPendingApprovals();
      if (generation !== requestGeneration || (publishOnlyWhenRetained && retainCount === 0)) {
        return;
      }
      emit(makeSnapshot(approvals, null, retainCount > 0));
      log('refresh succeeded approval_count=%d', approvals.length);
    } catch (error) {
      if (generation !== requestGeneration || (publishOnlyWhenRetained && retainCount === 0)) {
        return;
      }
      emit(makeSnapshot(snapshot.approvals, normalizeError(error), retainCount > 0));
      const errorType = error instanceof Error ? error.name : typeof error;
      log('refresh failed error_type=%s', errorType);
    } finally {
      if (activeRequestId === requestId) {
        activeRequestId = null;
        inFlight = null;
      }
      scheduleNextPoll(generation);
    }
  })();
  inFlight = request;
  return request;
}

export function refreshFlowPendingApprovals(): Promise<void> {
  if (!inFlight) return startRefresh();
  if (queuedRefresh) return queuedRefresh;

  clearPollTimer();
  const generation = requestGeneration;
  const activeRequest = inFlight;
  const queuedWhileRetained = retainCount > 0;
  const queued: Promise<void> = activeRequest.then(() => {
    if (queuedRefresh === queued) queuedRefresh = null;
    if (generation !== requestGeneration || (queuedWhileRetained && retainCount === 0)) {
      return;
    }
    return startRefresh();
  });
  queuedRefresh = queued;
  return queued;
}

export function retainFlowPendingApprovalsPolling(): () => void {
  retainCount += 1;
  if (retainCount === 1) {
    emit(makeSnapshot(snapshot.approvals, snapshot.error, true));
    if (!inFlight) void startRefresh(true);
  }

  let released = false;
  return () => {
    if (released) return;
    released = true;
    retainCount = Math.max(0, retainCount - 1);
    if (retainCount > 0) return;

    clearPollTimer();
    emit(makeSnapshot(snapshot.approvals, snapshot.error, false));
  };
}

export function useFlowPendingApprovalsSource(enabled: boolean): FlowPendingApprovalsSnapshot {
  const subscribe = useCallback(
    (listener: () => void) => (enabled ? subscribeFlowPendingApprovals(listener) : () => undefined),
    [enabled]
  );
  const getSnapshot = useCallback(
    () => (enabled ? getFlowPendingApprovalsSnapshot() : INITIAL_SNAPSHOT),
    [enabled]
  );
  const current = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => {
    if (!enabled) return;
    return retainFlowPendingApprovalsPolling();
  }, [enabled]);

  return current;
}

export function resetFlowPendingApprovalsStoreForTests(): void {
  clearPollTimer();
  requestGeneration += 1;
  activeRequestId = null;
  retainCount = 0;
  inFlight = null;
  queuedRefresh = null;
  snapshot = INITIAL_SNAPSHOT;
  listeners.clear();
}
