import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import authReducer, {
  clearToken,
  setAnalyticsForUser,
  setEncryptionKeyForUser,
  setOnboardedForUser,
  setOnboardingTasksForUser,
  setPrimaryWalletAddressForUser,
  setToken,
} from '../authSlice';
import teamReducer from '../teamSlice';
import userReducer from '../userSlice';

function createStore(preloaded?: Record<string, unknown>) {
  return configureStore({
    // Cast reducer map to any to avoid strict typing issues in this lightweight test harness.
    reducer: { auth: authReducer, user: userReducer, team: teamReducer } as unknown as any,
    preloadedState: preloaded as never,
  });
}

describe('authSlice', () => {
  it('has correct initial state', () => {
    const store = createStore();
    const state = store.getState().auth;
    expect(state.token).toBeNull();
    expect(state.isOnboardedByUser).toEqual({});
    expect(state.onboardingTasksByUser).toEqual({});
    expect(state.hasIncompleteOnboardingByUser).toEqual({});
    expect(state.isAnalyticsEnabledByUser).toEqual({});
  });

  it('sets token via setToken', () => {
    const store = createStore();
    store.dispatch(setToken('jwt-abc-123'));
    expect(store.getState().auth.token).toBe('jwt-abc-123');
  });

  it('sets onboarded flag per user', () => {
    const store = createStore();
    store.dispatch(setOnboardedForUser({ userId: 'u1', value: true }));
    expect(store.getState().auth.isOnboardedByUser.u1).toBe(true);

    store.dispatch(setOnboardedForUser({ userId: 'u2', value: false }));
    expect(store.getState().auth.isOnboardedByUser.u2).toBe(false);
    // u1 should be unaffected
    expect(store.getState().auth.isOnboardedByUser.u1).toBe(true);
  });

  it('sets analytics consent per user', () => {
    const store = createStore();
    store.dispatch(setAnalyticsForUser({ userId: 'u1', enabled: true }));
    expect(store.getState().auth.isAnalyticsEnabledByUser.u1).toBe(true);

    store.dispatch(setAnalyticsForUser({ userId: 'u1', enabled: false }));
    expect(store.getState().auth.isAnalyticsEnabledByUser.u1).toBe(false);
  });

  it('tracks onboarding tasks and incomplete flag per user', () => {
    const store = createStore();
    store.dispatch(
      setOnboardingTasksForUser({
        userId: 'u1',
        tasks: {
          accessibilityPermissionGranted: false,
          localModelConsentGiven: true,
          localModelDownloadStarted: false,
          enabledTools: [],
          connectedSources: [],
        },
      })
    );

    expect(store.getState().auth.hasIncompleteOnboardingByUser.u1).toBe(true);
    expect(store.getState().auth.onboardingTasksByUser.u1.localModelConsentGiven).toBe(true);

    store.dispatch(
      setOnboardingTasksForUser({
        userId: 'u1',
        tasks: {
          accessibilityPermissionGranted: true,
          localModelConsentGiven: true,
          localModelDownloadStarted: true,
          enabledTools: ['shell', 'file_read'],
          connectedSources: ['telegram'],
        },
      })
    );
    expect(store.getState().auth.hasIncompleteOnboardingByUser.u1).toBe(false);
  });

  it('clearToken resets all per-user state', async () => {
    const store = createStore();

    // Populate all per-user fields
    store.dispatch(setToken('jwt-abc'));
    store.dispatch(setOnboardedForUser({ userId: 'u1', value: true }));
    store.dispatch(setAnalyticsForUser({ userId: 'u1', enabled: true }));
    store.dispatch(setEncryptionKeyForUser({ userId: 'u1', key: 'aes-hex' }));
    store.dispatch(setPrimaryWalletAddressForUser({ userId: 'u1', address: '0xabc' }));
    store.dispatch(
      setOnboardingTasksForUser({
        userId: 'u1',
        tasks: {
          accessibilityPermissionGranted: true,
          localModelConsentGiven: true,
          localModelDownloadStarted: true,
          enabledTools: ['shell'],
          connectedSources: ['telegram'],
        },
      })
    );

    // Verify state is populated
    const before = store.getState().auth;
    expect(before.token).toBe('jwt-abc');
    expect(before.isOnboardedByUser.u1).toBe(true);
    expect(before.isAnalyticsEnabledByUser.u1).toBe(true);
    expect(before.encryptionKeyByUser.u1).toBe('aes-hex');

    // Dispatch clearToken thunk
    await store.dispatch(clearToken());

    // Verify everything is reset
    const after = store.getState().auth;
    expect(after.token).toBeNull();
    expect(after.isOnboardedByUser).toEqual({});
    expect(after.onboardingTasksByUser).toEqual({});
    expect(after.hasIncompleteOnboardingByUser).toEqual({});
    expect(after.isAnalyticsEnabledByUser).toEqual({});
    expect(after.encryptionKeyByUser).toEqual({});
    expect(after.primaryWalletAddressByUser).toEqual({});
  });
});
