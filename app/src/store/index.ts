import { configureStore, type Middleware } from '@reduxjs/toolkit';
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

import { DEV_JWT_TOKEN, IS_DEV } from '../utils/config';
import {
  logout as clearRustSession,
  storeSession,
  syncMemoryClientToken,
} from '../utils/tauriCommands';
import accessibilityReducer from './accessibilitySlice';
import aiReducer from './aiSlice';
import authReducer, { setOnboardedForUser, setToken } from './authSlice';
import channelConnectionsReducer from './channelConnectionsSlice';
import daemonReducer from './daemonSlice';
import intelligenceReducer from './intelligenceSlice';
import inviteReducer from './inviteSlice';
import socketReducer from './socketSlice';
import teamReducer from './teamSlice';
import threadReducer from './threadSlice';
import userReducer from './userSlice';

// Persist config for auth only
const authPersistConfig = {
  key: 'auth',
  storage,
  whitelist: [
    'token',
    'isOnboardedByUser',
    'onboardingTasksByUser',
    'hasIncompleteOnboardingByUser',
    'isAnalyticsEnabledByUser',
    'encryptionKeyByUser',
    'primaryWalletAddressByUser',
    'onboardingDeferredByUser',
  ],
};

// Persist config for AI state (config only)
const aiPersistConfig = { key: 'ai', storage, whitelist: ['config'] };

// Persist config for thread data and UI prefs (includes threads and messages)
// Note: activeThreadId is intentionally excluded as it's transient state
const threadPersistConfig = {
  key: 'thread',
  storage,
  whitelist: ['panelWidth', 'lastViewedAt', 'threads', 'messagesByThreadId', 'selectedThreadId'],
};

const persistedAuthReducer = persistReducer(authPersistConfig, authReducer);
const persistedAiReducer = persistReducer(aiPersistConfig, aiReducer);
const persistedThreadReducer = persistReducer(threadPersistConfig, threadReducer);
const channelConnectionsPersistConfig = {
  key: 'channelConnections',
  storage,
  whitelist: ['schemaVersion', 'migrationCompleted', 'defaultMessagingChannel', 'connections'],
};
const persistedChannelConnectionsReducer = persistReducer(
  channelConnectionsPersistConfig,
  channelConnectionsReducer
);

/**
 * Middleware that syncs the JWT token to the Rust SESSION_SERVICE whenever
 * setToken is dispatched or auth state is rehydrated from persist.
 */
const syncTokenToRust: Middleware = () => {
  let lastSyncedToken: string | null = null;
  return next => action => {
    const result = next(action);

    const syncToken = (token: string) => {
      if (token === lastSyncedToken) return;
      lastSyncedToken = token;

      // Pass a minimal user object — the token is what matters for SESSION_SERVICE
      storeSession(token, { id: '' }).catch(err =>
        console.warn('[syncTokenToRust] Failed to sync token:', err)
      );
      syncMemoryClientToken(token).catch(err =>
        console.warn('[syncTokenToRust] Failed to sync memory token:', err)
      );
    };

    // Sync on explicit setToken
    if (setToken.match(action) && action.payload) {
      syncToken(action.payload);
    }

    if ((action as { type?: string }).type === 'auth/_clearToken') {
      lastSyncedToken = null;
      clearRustSession().catch(err =>
        console.warn('[syncTokenToRust] Failed to clear core session:', err)
      );
    }

    // Sync on rehydration (app restart — persist loads token from localStorage)
    const a = action as { type?: string; key?: string; payload?: { token?: string } };
    if (a.type === REHYDRATE && a.key === 'auth') {
      const token = a.payload?.token;
      if (token) {
        syncToken(token);
      } else {
        lastSyncedToken = null;
      }
    }

    return result;
  };
};

export const store = configureStore({
  reducer: {
    auth: persistedAuthReducer,
    socket: socketReducer,
    user: userReducer,
    daemon: daemonReducer,
    ai: persistedAiReducer,
    team: teamReducer,
    thread: persistedThreadReducer,
    intelligence: intelligenceReducer,
    invite: inviteReducer,
    accessibility: accessibilityReducer,
    channelConnections: persistedChannelConnectionsReducer,
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
  // Dev-only: auto-inject JWT token for local testing without login flow.
  if (DEV_JWT_TOKEN && !store.getState().auth.token) {
    store.dispatch(setToken(DEV_JWT_TOKEN));
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
