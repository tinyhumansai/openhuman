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
import aiReducer from './aiSlice';
import authReducer from './authSlice';
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
