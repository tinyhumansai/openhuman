import { createSlice } from '@reduxjs/toolkit';

/**
 * Tracks whether this user has dismissed (or acted on) the in-app "Star us on
 * GitHub" CTA (#5005). Once `dismissed` is true the CTA never shows again.
 *
 * Persisted through `userScopedStorage` (see store/index.ts, whitelist
 * `['dismissed']`) so the choice is per-user and survives reloads/restarts —
 * a durable dismissal, not a process-local one. Both the "Star" and the
 * "Not now" actions flip this flag: a user who starred does not want the nudge
 * to keep reappearing either.
 */
export interface GithubStarState {
  dismissed: boolean;
}

const initialState: GithubStarState = { dismissed: false };

const githubStarSlice = createSlice({
  name: 'githubStar',
  initialState,
  reducers: {
    dismissGithubStarCta(state) {
      state.dismissed = true;
    },
  },
});

export const { dismissGithubStarCta } = githubStarSlice.actions;
export default githubStarSlice.reducer;
