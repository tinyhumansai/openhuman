import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { resetUserScopedState } from './resetActions';

// ── Types matching Rust `desktop_companion/types.rs` ─────────────────

/** Matches `CompanionState` enum (serde rename_all = "snake_case"). */
export type CompanionState = 'idle' | 'listening' | 'thinking' | 'speaking' | 'pointing' | 'error';

/** Matches `CompanionConfig` struct. */
export interface CompanionConfig {
  hotkey: string;
  activation_mode: string;
  ttl_secs: number;
  capture_screen: boolean;
  include_app_context: boolean;
}

/** Matches `CompanionStateChangedEvent` — Socket.IO payload. */
export interface CompanionStateChangedEvent {
  session_id: string;
  state: CompanionState;
  previous_state: CompanionState;
  message?: string;
}

/** Matches `CompanionSessionStatus` — RPC response. */
export interface CompanionSessionStatus {
  active: boolean;
  state: CompanionState;
  session_id: string | null;
  started_at_ms: number | null;
  expires_at_ms: number | null;
  remaining_ms: number | null;
  turn_count: number;
  last_error: string | null;
}

/** Matches `StartCompanionSessionResult`. */
export interface StartCompanionSessionResult {
  session_id: string;
  state: CompanionState;
  expires_at_ms: number | null;
}

/** Matches `StopCompanionSessionResult`. */
export interface StopCompanionSessionResult {
  stopped: boolean;
  reason: string | null;
}

// ── Slice state ──────────────────────────────────────────────────────

export interface CompanionSliceState {
  sessionActive: boolean;
  state: CompanionState;
  sessionId: string | null;
  config: CompanionConfig | null;
  lastError: string | null;
}

const initialState: CompanionSliceState = {
  sessionActive: false,
  state: 'idle',
  sessionId: null,
  config: null,
  lastError: null,
};

// ── Slice ────────────────────────────────────────────────────────────

const companionSlice = createSlice({
  name: 'companion',
  initialState,
  reducers: {
    setCompanionState(state, action: PayloadAction<CompanionStateChangedEvent>) {
      const event = action.payload;
      state.state = event.state;
      state.sessionId = event.session_id;
      state.sessionActive = event.state !== 'idle' && event.state !== 'error';
      if (event.state === 'error') {
        if (event.message) state.lastError = event.message;
      } else {
        // Clear stale error once the session recovers to a healthy state.
        state.lastError = null;
      }
    },
    setSessionActive(state, action: PayloadAction<{ active: boolean; sessionId: string | null }>) {
      state.sessionActive = action.payload.active;
      state.sessionId = action.payload.sessionId;
      if (!action.payload.active) {
        state.state = 'idle';
      }
    },
    setConfig(state, action: PayloadAction<CompanionConfig>) {
      state.config = action.payload;
    },
    setLastError(state, action: PayloadAction<string | null>) {
      state.lastError = action.payload;
    },
    clearSession() {
      return { ...initialState };
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
  },
});

export const { setCompanionState, setSessionActive, setConfig, setLastError, clearSession } =
  companionSlice.actions;

// ── Selectors ────────────────────────────────────────────────────────
// Selectors tolerate a missing `companion` slice so consumers (e.g. BottomTabBar)
// don't crash inside test harnesses that mock the store without this slice.

type MaybeCompanionRoot = { companion?: CompanionSliceState };

export const selectCompanionState = (state: MaybeCompanionRoot) =>
  state.companion?.state ?? initialState.state;

export const selectCompanionSessionActive = (state: MaybeCompanionRoot) =>
  state.companion?.sessionActive ?? initialState.sessionActive;

export const selectCompanionConfig = (state: MaybeCompanionRoot) =>
  state.companion?.config ?? initialState.config;

export const selectCompanionLastError = (state: MaybeCompanionRoot) =>
  state.companion?.lastError ?? initialState.lastError;

export default companionSlice.reducer;
