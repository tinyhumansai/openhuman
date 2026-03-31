import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import authReducer, {
  setAnalyticsForUser,
  setOnboardedForUser,
  setOnboardingTasksForUser,
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
});
