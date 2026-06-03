import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { resetUserScopedState } from './resetActions';

/**
 * PTT (Push-to-Talk) slice — persisted hotkey binding + session settings,
 * plus a non-persisted runtime hold flag that tracks whether the key is
 * currently held. The boot hook (Task 11) resets `isHeld` to false on mount
 * so a stale persisted value can never leave the app stuck in "held" mode.
 */

export interface PttState {
  /** Currently-bound PTT hotkey string (e.g. "F13" or "Ctrl+Alt+T"). null = unbound. */
  shortcut: string | null;
  /** When true, the agent's reply is spoken via TTS. */
  speakReplies: boolean;
  /** When true, the overlay window is shown during a PTT session. */
  showOverlay: boolean;
  /** Non-persisted runtime flag: is the PTT key currently held? */
  isHeld: boolean;
}

export const initialPttState: PttState = {
  shortcut: null,
  speakReplies: true,
  showOverlay: true,
  isHeld: false,
};

const pttSlice = createSlice({
  name: 'ptt',
  initialState: initialPttState,
  reducers: {
    setPttShortcut(state, action: PayloadAction<string | null>) {
      state.shortcut = action.payload;
    },
    setSpeakReplies(state, action: PayloadAction<boolean>) {
      state.speakReplies = action.payload;
    },
    setShowOverlay(state, action: PayloadAction<boolean>) {
      state.showOverlay = action.payload;
    },
    setIsHeld(state, action: PayloadAction<boolean>) {
      state.isHeld = action.payload;
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialPttState);
  },
});

export const { setPttShortcut, setSpeakReplies, setShowOverlay, setIsHeld } = pttSlice.actions;

// ── Selectors ────────────────────────────────────────────────────────────────
// Tolerate a missing `ptt` slice so consumers don't crash in test harnesses
// that mock the store without this slice.

type MaybePttRoot = { ptt?: PttState };

export const selectPttShortcut = (state: MaybePttRoot): string | null =>
  state.ptt?.shortcut ?? initialPttState.shortcut;

export const selectSpeakReplies = (state: MaybePttRoot): boolean =>
  state.ptt?.speakReplies ?? initialPttState.speakReplies;

export const selectShowOverlay = (state: MaybePttRoot): boolean =>
  state.ptt?.showOverlay ?? initialPttState.showOverlay;

export const selectIsHeld = (state: MaybePttRoot): boolean =>
  state.ptt?.isHeld ?? initialPttState.isHeld;

export const pttReducer = pttSlice.reducer;
export default pttSlice.reducer;
