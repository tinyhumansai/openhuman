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

export const selectPttShortcut = (state: { ptt: PttState }): string | null =>
  state.ptt.shortcut;

export const selectSpeakReplies = (state: { ptt: PttState }): boolean =>
  state.ptt.speakReplies;

export const selectShowOverlay = (state: { ptt: PttState }): boolean =>
  state.ptt.showOverlay;

export const selectIsHeld = (state: { ptt: PttState }): boolean =>
  state.ptt.isHeld;

export const pttReducer = pttSlice.reducer;
