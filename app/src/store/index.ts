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

import { IS_DEV } from '../utils/config';
import accountsReducer from './accountsSlice';
import channelConnectionsReducer from './channelConnectionsSlice';
import chatRuntimeReducer from './chatRuntimeSlice';
import notificationReducer from './notificationSlice';
import providerSurfacesReducer from './providerSurfaceSlice';
import socketReducer from './socketSlice';
import threadReducer from './threadSlice';
import { userScopedStorage } from './userScopedStorage';

// Persisted slices write through `userScopedStorage` so each user's blob
// lives at `${userId}:persist:<key>` instead of a single per-device blob
// that leaks across users on logout/login (#900).
const storage = userScopedStorage;

const channelConnectionsPersistConfig = {
  key: 'channelConnections',
  storage,
  whitelist: ['schemaVersion', 'migrationCompleted', 'defaultMessagingChannel', 'connections'],
};
const persistedChannelConnectionsReducer = persistReducer(
  channelConnectionsPersistConfig,
  channelConnectionsReducer
);

// Persist only the account list (not the live message stream / logs which
// are re-ingested every time we open an account).
const accountsPersistConfig = {
  key: 'accounts',
  storage,
  whitelist: ['accounts', 'order', 'activeAccountId'],
};
const persistedAccountsReducer = persistReducer(accountsPersistConfig, accountsReducer);

const notificationPersistConfig = {
  key: 'notifications',
  storage,
  whitelist: ['items', 'preferences'],
};
const persistedNotificationReducer = persistReducer(notificationPersistConfig, notificationReducer);

export const store = configureStore({
  reducer: {
    socket: socketReducer,
    thread: threadReducer,
    chatRuntime: chatRuntimeReducer,
    channelConnections: persistedChannelConnectionsReducer,
    accounts: persistedAccountsReducer,
    notifications: persistedNotificationReducer,
    providerSurfaces: providerSurfacesReducer,
  },
  middleware: getDefaultMiddleware => {
    const middleware = getDefaultMiddleware({
      serializableCheck: { ignoredActions: [FLUSH, REHYDRATE, PAUSE, PERSIST, PURGE, REGISTER] },
    });

    // Add redux-logger in development with collapsed groups
    if (IS_DEV) {
      return middleware.concat(createLogger({ collapsed: true, duration: true, timestamp: true }));
    }
    return middleware;
  },
});

export const persistor = persistStore(store);

// Expose the store on `window` so WDIO E2E specs can read Redux state directly
// to assert backing-state changes (see app/test/e2e/specs/*.spec.ts). The store
// holds no secrets that aren't already in the renderer's memory; this only
// surfaces the existing handle under a stable, namespaced key.
if (typeof window !== 'undefined') {
  (window as unknown as { __OPENHUMAN_STORE__?: typeof store }).__OPENHUMAN_STORE__ = store;
}

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
