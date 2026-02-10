import type { Middleware } from '@reduxjs/toolkit';
import { configureStore } from '@reduxjs/toolkit';
import { createLogger } from 'redux-logger';
import {
  FLUSH,
  PAUSE,
  PERSIST,
  persistReducer,
  persistStore,
  PURGE,
  REGISTER,
  REHYDRATE,
} from 'redux-persist';
import storage from 'redux-persist/lib/storage';

import { IS_DEV } from '../utils/config';
import { storeSession } from '../utils/tauriCommands';
import aiReducer from './aiSlice';
import authReducer, { setOnboardedForUser, setToken } from './authSlice';
import skillsReducer from './skillsSlice';
import socketReducer from './socketSlice';
import teamReducer from './teamSlice';
import userReducer from './userSlice';

// Persist config for auth only
const authPersistConfig = {
  key: 'auth',
  storage,
  whitelist: ['token', 'isOnboardedByUser', 'isAnalyticsEnabledByUser'],
};

// Persist config for AI state (config only)
const aiPersistConfig = { key: 'ai', storage, whitelist: ['config'] };

// Persist config for skills state (setupComplete per skill)
const skillsPersistConfig = { key: 'skills', storage, whitelist: ['skills'] };

const persistedAuthReducer = persistReducer(authPersistConfig, authReducer);
const persistedAiReducer = persistReducer(aiPersistConfig, aiReducer);
const persistedSkillsReducer = persistReducer(skillsPersistConfig, skillsReducer);

/**
 * Middleware that syncs the JWT token to the Rust SESSION_SERVICE whenever
 * setToken is dispatched or auth state is rehydrated from persist.
 */
const syncTokenToRust: Middleware = () => next => action => {
  const result = next(action);

  const syncToken = (token: string) => {
    // Pass a minimal user object — the token is what matters for SESSION_SERVICE
    storeSession(token, { id: '' }).catch(err =>
      console.warn('[syncTokenToRust] Failed to sync token:', err)
    );
  };

  // Sync on explicit setToken
  if (setToken.match(action) && action.payload) {
    syncToken(action.payload);
  }

  // Sync on rehydration (app restart — persist loads token from localStorage)
  const a = action as { type?: string; key?: string; payload?: { token?: string } };
  if (a.type === REHYDRATE && a.key === 'auth') {
    const token = a.payload?.token;
    if (token) {
      syncToken(token);
    }
  }

  return result;
};

export const store = configureStore({
  reducer: {
    auth: persistedAuthReducer,
    socket: socketReducer,
    user: userReducer,
    ai: persistedAiReducer,
    skills: persistedSkillsReducer,
    team: teamReducer,
  },
  middleware: getDefaultMiddleware => {
    const middleware = getDefaultMiddleware({
      serializableCheck: { ignoredActions: [FLUSH, REHYDRATE, PAUSE, PERSIST, PURGE, REGISTER] },
    }).concat(syncTokenToRust);

    // Add redux-logger in development with collapsed groups
    if (IS_DEV) {
      return middleware.concat(createLogger({ collapsed: true, duration: true, timestamp: true }));
    }
    return middleware;
  },
});

export const persistor = persistStore(store, null, () => {
  // Dev-only: auto-inject JWT token for testing (e.g. Android without login flow)
  const devToken = import.meta.env.VITE_DEV_JWT_TOKEN;
  if (devToken && !store.getState().auth.token) {
    store.dispatch(setToken(devToken));
    console.log('[dev] Auto-injected JWT token from VITE_DEV_JWT_TOKEN');

    // Auto-mark user as onboarded once their profile is fetched
    const unsub = store.subscribe(() => {
      const state = store.getState();
      const userId = state.user.user?._id;
      if (userId && !state.auth.isOnboardedByUser[userId]) {
        store.dispatch(setOnboardedForUser({ userId, value: true }));
        console.log('[dev] Auto-marked user as onboarded:', userId);
        unsub();
      }
    });
  }
});

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
