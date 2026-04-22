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
import accountsReducer from './accountsSlice';
import channelConnectionsReducer from './channelConnectionsSlice';
import chatRuntimeReducer from './chatRuntimeSlice';
import notificationsReducer from './notificationsSlice';
import socketReducer from './socketSlice';
import threadReducer from './threadSlice';

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

export const store = configureStore({
  reducer: {
    socket: socketReducer,
    thread: threadReducer,
    chatRuntime: chatRuntimeReducer,
    channelConnections: persistedChannelConnectionsReducer,
    accounts: persistedAccountsReducer,
    notifications: notificationsReducer,
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

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
