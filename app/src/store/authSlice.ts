import { createAsyncThunk, createSlice, PayloadAction } from '@reduxjs/toolkit';

import { clearTeamState } from './teamSlice';
import { clearUser } from './userSlice';

export interface AuthState {
  token: string | null;
  /** True once startup auth/session bootstrap has checked core binary state */
  isAuthBootstrapComplete: boolean;
  /** Onboarding completion per user id */
  isOnboardedByUser: Record<string, boolean>;
  /** Additional onboarding task progress per user id */
  onboardingTasksByUser: Record<string, UserOnboardingTasks>;
  /** True when user completed onboarding route but skipped some optional setup tasks */
  hasIncompleteOnboardingByUser: Record<string, boolean>;
  /** Analytics consent per user id (opt-in during onboarding) */
  isAnalyticsEnabledByUser: Record<string, boolean>;
  /** AES encryption key (hex) derived from mnemonic, per user id */
  encryptionKeyByUser: Record<string, string>;
  /** Primary EVM wallet address (0x...) derived from mnemonic, per user id */
  primaryWalletAddressByUser: Record<string, string>;
  /** Timestamp when user deferred onboarding (value = epoch ms), per user id */
  onboardingDeferredByUser: Record<string, number>;
}

export interface UserOnboardingTasks {
  accessibilityPermissionGranted: boolean;
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  enabledTools: string[];
  connectedSources: string[];
  updatedAtMs: number;
}

const initialState: AuthState = {
  token: null,
  isAuthBootstrapComplete: false,
  isOnboardedByUser: {},
  onboardingTasksByUser: {},
  hasIncompleteOnboardingByUser: {},
  isAnalyticsEnabledByUser: {},
  encryptionKeyByUser: {},
  primaryWalletAddressByUser: {},
  onboardingDeferredByUser: {},
};

const authSlice = createSlice({
  name: 'auth',
  initialState,
  reducers: {
    setToken: (state, action: PayloadAction<string>) => {
      state.token = action.payload;
    },
    setAuthBootstrapComplete: (state, action: PayloadAction<boolean>) => {
      state.isAuthBootstrapComplete = action.payload;
    },
    _clearToken: state => {
      state.token = null;
      state.isOnboardedByUser = {};
      state.onboardingTasksByUser = {};
      state.hasIncompleteOnboardingByUser = {};
      state.isAnalyticsEnabledByUser = {};
      state.encryptionKeyByUser = {};
      state.primaryWalletAddressByUser = {};
      state.onboardingDeferredByUser = {};
    },
    setOnboardedForUser: (state, action: PayloadAction<{ userId: string; value: boolean }>) => {
      const { userId, value } = action.payload;
      state.isOnboardedByUser[userId] = value;
    },
    setAnalyticsForUser: (state, action: PayloadAction<{ userId: string; enabled: boolean }>) => {
      const { userId, enabled } = action.payload;
      state.isAnalyticsEnabledByUser[userId] = enabled;
    },
    setOnboardingTasksForUser: (
      state,
      action: PayloadAction<{ userId: string; tasks: Omit<UserOnboardingTasks, 'updatedAtMs'> }>
    ) => {
      const { userId, tasks } = action.payload;
      state.onboardingTasksByUser[userId] = { ...tasks, updatedAtMs: Date.now() };

      const hasIncomplete =
        !tasks.accessibilityPermissionGranted || tasks.connectedSources.length === 0;
      state.hasIncompleteOnboardingByUser[userId] = hasIncomplete;
    },
    setEncryptionKeyForUser: (state, action: PayloadAction<{ userId: string; key: string }>) => {
      const { userId, key } = action.payload;
      state.encryptionKeyByUser[userId] = key;
    },
    setPrimaryWalletAddressForUser: (
      state,
      action: PayloadAction<{ userId: string; address: string }>
    ) => {
      const { userId, address } = action.payload;
      state.primaryWalletAddressByUser[userId] = address;
    },
    setOnboardingDeferredForUser: (
      state,
      action: PayloadAction<{ userId: string; deferred: boolean }>
    ) => {
      const { userId, deferred } = action.payload;
      if (deferred) {
        state.onboardingDeferredByUser[userId] = Date.now();
      } else {
        delete state.onboardingDeferredByUser[userId];
      }
    },
  },
});

// Thunk that clears both token and user data
export const clearToken = createAsyncThunk('auth/clearToken', async (_, { dispatch }) => {
  dispatch(authSlice.actions._clearToken());
  dispatch(clearUser());
  dispatch(clearTeamState());
});

export const {
  setToken,
  setAuthBootstrapComplete,
  setOnboardedForUser,
  setAnalyticsForUser,
  setOnboardingTasksForUser,
  setEncryptionKeyForUser,
  setPrimaryWalletAddressForUser,
  setOnboardingDeferredForUser,
} = authSlice.actions;
export default authSlice.reducer;
