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
import channelConnectionsReducer from './channelConnectionsSlice';
import socketReducer from './socketSlice';
import threadReducer from './threadSlice';

// Persist config for thread data and UI prefs (includes threads and messages)
// Note: activeThreadId is intentionally excluded as it's transient state
const threadPersistConfig = {
  key: 'thread',
  storage,
  whitelist: ['panelWidth', 'lastViewedAt', 'threads', 'messagesByThreadId', 'selectedThreadId'],
};

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

export const store = configureStore({
  reducer: {
    socket: socketReducer,
    thread: persistedThreadReducer,
    channelConnections: persistedChannelConnectionsReducer,
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
