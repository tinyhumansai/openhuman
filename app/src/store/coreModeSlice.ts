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
import { REHYDRATE } from 'redux-persist';

import { E2E_DEFAULT_CORE_MODE } from '../utils/config';
import { normalizeRpcUrl } from '../utils/configPersistence';

export type CoreMode =
  | { kind: 'unset' }
  | { kind: 'local' }
  | {
      kind: 'cloud';
      url: string;
      /**
       * Bearer token for the remote core. Cloud cores require auth (see
       * `OPENHUMAN_CORE_TOKEN` in docs/CLOUD_DEPLOY.md). Optional in the type
       * so persisted state from older builds (which stored cloud mode without
       * a token) still hydrates; the BootCheckGate picker requires a value.
       */
      token?: string;
    };

export interface CoreModeState {
  mode: CoreMode;
}

/** Synchronous localStorage keys mirrored by `configPersistence.ts`. */
const RPC_URL_STORAGE_KEY = 'openhuman_core_rpc_url';
const CORE_TOKEN_STORAGE_KEY = 'openhuman_core_rpc_token';
const CORE_MODE_STORAGE_KEY = 'openhuman_core_mode';

/**
 * Derive the initial mode synchronously from `localStorage`.
 *
 * redux-persist saves slice state asynchronously (debounced). When the app
 * reloads (e.g. `handleIdentityFlip` → `restartApp` after the cloud core
 * returns a logged-in user that doesn't match the device's seed), the
 * persisted `coreMode` blob may not have been flushed before the reload.
 * Falling back to plain unset would put the user back on the picker even
 * though they just chose cloud, producing an infinite picker → reload loop.
 *
 * The picker writes `openhuman_core_rpc_url`, `openhuman_core_rpc_token`,
 * and `openhuman_core_mode` synchronously before any async dispatch, so we
 * can recover the exact mode on reload regardless of the persist flush race.
 */
function deriveInitialMode(): CoreMode {
  if (E2E_DEFAULT_CORE_MODE === 'local') {
    return { kind: 'local' };
  }

  if (typeof localStorage === 'undefined') return { kind: 'unset' };
  try {
    const mode = localStorage.getItem(CORE_MODE_STORAGE_KEY)?.trim();
    if (mode === 'local') return { kind: 'local' };
    if (mode === 'cloud') {
      const url = localStorage.getItem(RPC_URL_STORAGE_KEY)?.trim();
      const token = localStorage.getItem(CORE_TOKEN_STORAGE_KEY)?.trim();
      if (url && token) return { kind: 'cloud', url: normalizeRpcUrl(url), token };
    }
  } catch {
    /* localStorage unavailable — fall through to unset */
  }
  return { kind: 'unset' };
}

const initialState: CoreModeState = { mode: deriveInitialMode() };

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
  extraReducers: builder => {
    builder.addCase(REHYDRATE, (state, action) => {
      const rehydrateAction = action as typeof action & { key?: string };
      if (rehydrateAction.key !== 'coreMode') return;

      // The plain marker is written synchronously before boot-check work can
      // reload the renderer. Let it win over stale async redux-persist blobs.
      const synchronousMode = deriveInitialMode();
      if (synchronousMode.kind !== 'unset') {
        state.mode = synchronousMode;
      }
    });
  },
});

export const { setCoreMode, resetCoreMode } = coreModeSlice.actions;
export default coreModeSlice.reducer;
