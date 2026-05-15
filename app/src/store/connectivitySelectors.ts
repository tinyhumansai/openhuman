import { RootState } from './index';

/**
 * Single app-level "what is broken right now?" derived state. Order matters —
 * the user-blocking outage wins over the soft "we're reconnecting" state.
 *
 * - `internet-offline`  : navigator.onLine = false. Nothing else can talk.
 * - `core-unreachable`  : local sidecar isn't answering. App is dead-in-the-water.
 * - `backend-only`      : backend Socket.IO is down but core is alive — the
 *                         app stays usable, we just show a soft banner.
 * - `ok`                : everything healthy.
 */
export type BlockingState = 'internet-offline' | 'core-unreachable' | 'backend-only' | 'ok';

export const selectInternet = (s: RootState) => s.connectivity.internet;
export const selectCore = (s: RootState) => s.connectivity.core;
export const selectBackend = (s: RootState) => s.connectivity.backend;
export const selectConnectivityErrors = (s: RootState) => s.connectivity.lastError;

export const selectBlockingState = (s: RootState): BlockingState => {
  if (s.connectivity.internet === 'offline') return 'internet-offline';
  if (s.connectivity.core === 'unreachable') return 'core-unreachable';
  if (s.connectivity.backend === 'disconnected' || s.connectivity.backend === 'connecting') {
    return 'backend-only';
  }
  return 'ok';
};
