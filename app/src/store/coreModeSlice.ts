/**
 * coreModeSlice — persists the user's chosen core connection mode across
 * launches.  Two kinds of mode exist:
 *
 *   local  — embedded in-process core; spawned by the Tauri shell on demand.
 *   cloud  — user-supplied HTTP(S) URL to a remote core RPC endpoint.
 *
 * `unset` is the initial value shown to first-time users; the BootCheckGate
 * forces the user to pick before the rest of the app mounts.  After that the
 * value is persisted in plain localStorage (NOT user-scoped storage) because
 * it is pre-login and not tied to any particular user identity.
 */
import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

export type CoreMode = { kind: 'unset' } | { kind: 'local' } | { kind: 'cloud'; url: string };

export interface CoreModeState {
  mode: CoreMode;
}

const initialState: CoreModeState = { mode: { kind: 'unset' } };

const coreModeSlice = createSlice({
  name: 'coreMode',
  initialState,
  reducers: {
    /**
     * Set the active core mode.  Dispatched by the BootCheckGate picker when
     * the user clicks "Continue".
     */
    setCoreMode(state, action: PayloadAction<CoreMode>) {
      state.mode = action.payload;
    },

    /**
     * Reset back to `unset` so the picker re-appears on the next render.
     * Dispatched by "Switch mode" affordances inside the gate.
     */
    resetCoreMode(state) {
      state.mode = { kind: 'unset' };
    },
  },
});

export const { setCoreMode, resetCoreMode } = coreModeSlice.actions;
export default coreModeSlice.reducer;
