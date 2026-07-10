import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

/**
 * Tracks which announcements this user has already seen so the harness-init
 * banner shows each one exactly once. Persisted through `userScopedStorage`
 * (see store/index.ts) so the seen set is per-user and survives reloads.
 */
export interface AnnouncementState {
  shownIds: string[];
}

const initialState: AnnouncementState = { shownIds: [] };

const MAX_TRACKED = 200;

const announcementSlice = createSlice({
  name: 'announcement',
  initialState,
  reducers: {
    markAnnouncementShown(state, action: PayloadAction<string>) {
      const id = action.payload;
      if (!id || state.shownIds.includes(id)) {
        return;
      }
      state.shownIds.push(id);
      // Cap the history so a long-lived install can't grow it unbounded; the
      // oldest ids fall off first (they're the least likely to reappear).
      if (state.shownIds.length > MAX_TRACKED) {
        state.shownIds.splice(0, state.shownIds.length - MAX_TRACKED);
      }
    },
  },
});

export const { markAnnouncementShown } = announcementSlice.actions;
export default announcementSlice.reducer;
