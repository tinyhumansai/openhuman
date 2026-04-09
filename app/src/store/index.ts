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

import type { User } from '../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../types/team';
import { IS_DEV } from '../utils/config';
import aiReducer from './aiSlice';
import type { AuthState } from './authSlice';
import channelConnectionsReducer from './channelConnectionsSlice';
import daemonReducer from './daemonSlice';
import type { IntelligenceState } from './intelligenceSlice';
import inviteReducer from './inviteSlice';
import socketReducer from './socketSlice';
import threadReducer from './threadSlice';
import webhooksReducer from './webhooksSlice';

// Persist config for AI state (config only)
const aiPersistConfig = { key: 'ai', storage, whitelist: ['config'] };

// Persist config for thread data and UI prefs (includes threads and messages)
// Note: activeThreadId is intentionally excluded as it's transient state
const threadPersistConfig = {
  key: 'thread',
  storage,
  whitelist: ['panelWidth', 'lastViewedAt', 'threads', 'messagesByThreadId', 'selectedThreadId'],
};

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

export const store = configureStore({
  reducer: {
    socket: socketReducer,
    daemon: daemonReducer,
    ai: persistedAiReducer,
    thread: persistedThreadReducer,
    invite: inviteReducer,
    channelConnections: persistedChannelConnectionsReducer,
    webhooks: webhooksReducer,
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

type RuntimeRootState = ReturnType<typeof store.getState>;

type LegacyRootState = {
  auth: AuthState;
  user: { user: User | null; isLoading: boolean; error: string | null };
  team: {
    teams: TeamWithRole[];
    members: TeamMember[];
    invites: TeamInvite[];
    isLoading: boolean;
    isLoadingMembers: boolean;
    isLoadingInvites: boolean;
    error: string | null;
  };
  intelligence: IntelligenceState;
};

export type RootState = RuntimeRootState & LegacyRootState;
export type AppDispatch = typeof store.dispatch;
