import { createSlice, PayloadAction } from '@reduxjs/toolkit';

export interface HaltState {
  engaged: boolean;
  reason?: string;
  engaged_at_ms?: number;
  source?: string;
}

export interface SafetyState {
  halted: boolean;
  reason?: string;
  since?: number;
  source?: string;
}

const initialState: SafetyState = { halted: false };

const safetySlice = createSlice({
  name: 'safety',
  initialState,
  reducers: {
    setHalt(_state, action: PayloadAction<{ reason?: string; source?: string; since?: number }>) {
      return {
        halted: true,
        reason: action.payload.reason,
        source: action.payload.source,
        since: action.payload.since,
      };
    },
    clearHalt() {
      return { halted: false };
    },
    hydrateHalt(_state, action: PayloadAction<HaltState>) {
      const h = action.payload;
      return h.engaged
        ? { halted: true, reason: h.reason, source: h.source, since: h.engaged_at_ms }
        : { halted: false };
    },
  },
});

export const { setHalt, clearHalt, hydrateHalt } = safetySlice.actions;
// Defensive reads: some App-shell tests mock the store with a partial state that
// omits the `safety` slice. Optional chaining keeps the kill-switch UI from
// crashing the shell in that case (halted → false, no banner).
export const selectHalted = (state: { safety?: SafetyState }) => state.safety?.halted ?? false;
export const selectHaltReason = (state: { safety?: SafetyState }) => state.safety?.reason;
export default safetySlice.reducer;
