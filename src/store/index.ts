import { configureStore } from "@reduxjs/toolkit";
import {
  persistStore,
  persistReducer,
  createTransform,
  FLUSH,
  REHYDRATE,
  PAUSE,
  PERSIST,
  PURGE,
  REGISTER,
} from "redux-persist";
import storage from "redux-persist/lib/storage";
import authReducer from "./authSlice";
import socketReducer from "./socketSlice";
import userReducer from "./userSlice";
import telegramReducer from "./telegram";
import { createLogger } from "redux-logger";
import { IS_DEV } from "../utils/config";
import type { TelegramRootState, TelegramState } from "./telegram/types";
import { initialState as telegramInitialState } from "./telegram/types";

// Persist config for auth only
const authPersistConfig = {
  key: "auth",
  storage,
  whitelist: ["token", "isOnboardedByUser"],
};

// Strip volatile runtime fields from each per-user Telegram state on rehydrate.
// These fields reflect in-memory MTProto client state and must start fresh on reload.
const telegramVolatileTransform = createTransform<
  TelegramRootState["byUser"],
  TelegramRootState["byUser"]
>(
  // inbound (state -> storage): pass through as-is
  (inboundState) => inboundState,
  // outbound (storage -> state): reset volatile fields per user
  (outboundState) => {
    const cleaned: Record<string, TelegramState> = {};
    for (const [userId, userState] of Object.entries(outboundState)) {
      cleaned[userId] = {
        ...userState,
        isInitialized: telegramInitialState.isInitialized,
        connectionStatus: telegramInitialState.connectionStatus,
        connectionError: telegramInitialState.connectionError,
        isLoadingChats: telegramInitialState.isLoadingChats,
        isLoadingMessages: telegramInitialState.isLoadingMessages,
        isLoadingThreads: telegramInitialState.isLoadingThreads,
        // Thread index is volatile — viewport/outlying state is runtime-only
        threadIndex: telegramInitialState.threadIndex,
      };
    }
    return cleaned;
  },
  { whitelist: ["byUser"] },
);

// Persist config for telegram state (scoped by user in byUser)
const telegramPersistConfig = {
  key: "telegram",
  storage,
  whitelist: ["byUser"],
  transforms: [telegramVolatileTransform],
};

const persistedAuthReducer = persistReducer(authPersistConfig, authReducer);
const persistedTelegramReducer = persistReducer(
  telegramPersistConfig,
  telegramReducer,
);

export const store = configureStore({
  reducer: {
    auth: persistedAuthReducer,
    socket: socketReducer,
    user: userReducer,
    telegram: persistedTelegramReducer,
  },
  middleware: (getDefaultMiddleware) => {
    const middleware = getDefaultMiddleware({
      serializableCheck: {
        ignoredActions: [FLUSH, REHYDRATE, PAUSE, PERSIST, PURGE, REGISTER],
      },
    });

    // Add redux-logger in development with collapsed groups
    if (IS_DEV) {
      return middleware.concat(
        createLogger({
          collapsed: true,
          duration: true,
          timestamp: true,
        }),
      );
    }
    return middleware;
  },
});

export const persistor = persistStore(store);

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
