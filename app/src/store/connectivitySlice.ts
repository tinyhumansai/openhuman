import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

/**
 * Three independent connectivity channels surfaced separately so the UI can
 * tell the user *which* link is broken instead of one conflated "Disconnected"
 * pill (#1527).
 *
 * - `internet` — browser navigator.onLine. Source of truth: `online`/`offline`
 *   listeners on `window`.
 * - `core`     — local Rust sidecar reachability. Source: `coreHealthMonitor`
 *   poll of `openhuman.connectivity_diag`.
 * - `backend`  — Socket.IO link to the hosted backend. Source:
 *   `socketService` lifecycle callbacks.
 */

export type InternetState = 'online' | 'offline';
export type CoreState = 'reachable' | 'unreachable' | 'unknown';
export type BackendState = 'connected' | 'disconnected' | 'connecting';

export interface ConnectivityState {
  internet: InternetState;
  core: CoreState;
  backend: BackendState;
  /**
   * Last error string emitted per channel, if any. Cleared on the next
   * successful state for that channel. UI surfaces these in tooltips /
   * blocking screens for diagnosability.
   */
  lastError: { internet?: string; core?: string; backend?: string };
}

const initialState: ConnectivityState = {
  internet: typeof navigator !== 'undefined' && navigator.onLine === false ? 'offline' : 'online',
  core: 'unknown',
  backend: 'connecting',
  lastError: {},
};

const slice = createSlice({
  name: 'connectivity',
  initialState,
  reducers: {
    setInternet(state, action: PayloadAction<{ value: InternetState; error?: string }>) {
      state.internet = action.payload.value;
      if (action.payload.value === 'online') {
        delete state.lastError.internet;
      } else {
        state.lastError.internet = action.payload.error;
      }
    },
    setCore(state, action: PayloadAction<{ value: CoreState; error?: string }>) {
      state.core = action.payload.value;
      if (action.payload.value === 'reachable') {
        delete state.lastError.core;
      } else {
        state.lastError.core = action.payload.error;
      }
    },
    setBackend(state, action: PayloadAction<{ value: BackendState; error?: string }>) {
      state.backend = action.payload.value;
      if (action.payload.value === 'connected' || action.payload.value === 'connecting') {
        // Clear the stale error on both successful connection and reconnect
        // attempts. Previously only 'connected' deleted lastError.backend,
        // which meant a prior disconnect error (e.g. "transport close") was
        // left set to `undefined` — not deleted — during 'connecting'. UI
        // components that read lastError.backend could still surface a stale
        // message in the reconnect window even though the key appeared falsy.
        delete state.lastError.backend;
      } else {
        state.lastError.backend = action.payload.error;
      }
    },
  },
});

export const { setInternet, setCore, setBackend } = slice.actions;
export default slice.reducer;
