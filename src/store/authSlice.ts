import { createAsyncThunk, createSlice, PayloadAction } from '@reduxjs/toolkit';

import { clearTeamState } from './teamSlice';
import { clearUser } from './userSlice';

export interface AuthState {
  token: string | null;
  /** Onboarding completion per user id */
  isOnboardedByUser: Record<string, boolean>;
  /** Analytics consent per user id (opt-in during onboarding) */
  isAnalyticsEnabledByUser: Record<string, boolean>;
}

const initialState: AuthState = {
  token: null,
  isOnboardedByUser: {},
  isAnalyticsEnabledByUser: {},
};

const authSlice = createSlice({
  name: 'auth',
  initialState,
  reducers: {
    setToken: (state, action: PayloadAction<string>) => {
      state.token = action.payload;
    },
    _clearToken: state => {
      state.token = null;
    },
    setOnboardedForUser: (state, action: PayloadAction<{ userId: string; value: boolean }>) => {
      const { userId, value } = action.payload;
      state.isOnboardedByUser[userId] = value;
    },
    setAnalyticsForUser: (state, action: PayloadAction<{ userId: string; enabled: boolean }>) => {
      const { userId, enabled } = action.payload;
      state.isAnalyticsEnabledByUser[userId] = enabled;
    },
  },
});

// Thunk that clears both token and user data
export const clearToken = createAsyncThunk('auth/clearToken', async (_, { dispatch }) => {
  dispatch(authSlice.actions._clearToken());
  dispatch(clearUser());
  dispatch(clearTeamState());
});

export const { setToken, setOnboardedForUser, setAnalyticsForUser } = authSlice.actions;
export default authSlice.reducer;
