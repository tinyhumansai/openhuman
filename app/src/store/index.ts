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

import { E2E_RESTART_APP_AS_RELOAD, IS_DEV } from '../utils/config';
import accountsReducer from './accountsSlice';
import agentProfileReducer from './agentProfileSlice';
import channelConnectionsReducer from './channelConnectionsSlice';
import chatRuntimeReducer from './chatRuntimeSlice';
import companionReducer from './companionSlice';
import connectivityReducer from './connectivitySlice';
import coreModeReducer from './coreModeSlice';
import localeReducer from './localeSlice';
import mascotReducer from './mascotSlice';
import notificationReducer from './notificationSlice';
import personaReducer from './personaSlice';
import providerSurfacesReducer from './providerSurfaceSlice';
import socketReducer from './socketSlice';
import themeReducer from './themeSlice';
import threadReducer from './threadSlice';
import { userScopedStorage } from './userScopedStorage';
import workflowsReducer from './workflowsSlice';

// Persisted slices write through `userScopedStorage` so each user's blob
// lives at `${userId}:persist:<key>` instead of a single per-device blob
// that leaks across users on logout/login (#900).
const storage = userScopedStorage;

// coreMode is pre-login and not user-scoped — use plain localStorage so the
// setting survives across user switches without leaking per-user state.
// Inline adapter rather than `redux-persist/lib/storage`'s default export,
// which Vite's CJS dep-pre-bundling can resolve to the module namespace
// (then `storage.getItem` is undefined and rehydrate throws on cold boot).
const localStorageAdapter = {
  getItem: (key: string) =>
    Promise.resolve(
      (() => {
        try {
          return localStorage.getItem(key);
        } catch {
          return null;
        }
      })()
    ),
  setItem: (key: string, value: string) =>
    Promise.resolve(
      (() => {
        try {
          localStorage.setItem(key, value);
        } catch {
          /* ignore quota / unavailable */
        }
      })()
    ),
  removeItem: (key: string) =>
    Promise.resolve(
      (() => {
        try {
          localStorage.removeItem(key);
        } catch {
          /* ignore */
        }
      })()
    ),
};
const coreModePersistConfig = {
  key: 'coreMode',
  storage: localStorageAdapter,
  whitelist: ['mode'],
};
const persistedCoreModeReducer = persistReducer(coreModePersistConfig, coreModeReducer);

const localePersistConfig = { key: 'locale', storage: localStorageAdapter, whitelist: ['current'] };
const persistedLocaleReducer = persistReducer(localePersistConfig, localeReducer);

// Theme preference is pre-login and applies to the whole desktop app
// (light/dark/system). Persist via plain localStorage so it survives user
// switches like coreMode does.
const themePersistConfig = {
  key: 'theme',
  storage: localStorageAdapter,
  whitelist: ['mode', 'tabBarLabels'],
};
const persistedThemeReducer = persistReducer(themePersistConfig, themeReducer);

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
//
// Issue #2044 — `activeAccountId` is deliberately NOT persisted. It is a
// per-session UX selection: persisting it caused provider webviews to
// auto-surface on dev hot reload / app restart without an explicit user
// click, because `Accounts.tsx` immediately mounts `WebviewHost` for the
// active account and `WebviewHost` calls `openWebviewAccount` on mount.
// `lastActiveAccountId` is still persisted so the off-screen MRU prewarm
// can warm the same account in the background — that webview stays
// hidden until the user clicks the rail.
const accountsPersistConfig = {
  key: 'accounts',
  storage,
  whitelist: ['accounts', 'order', 'lastActiveAccountId'],
};
const persistedAccountsReducer = persistReducer(accountsPersistConfig, accountsReducer);

const notificationPersistConfig = {
  key: 'notifications',
  storage,
  whitelist: ['items', 'preferences'],
};
const persistedNotificationReducer = persistReducer(notificationPersistConfig, notificationReducer);

// Persist only the user's last-viewed thread id so a reload resumes where
// they were instead of falling through to "create a new thread". The
// thread list and per-thread message caches are re-fetched from the core
// on boot, so we deliberately don't persist them.
const threadPersistConfig = { key: 'thread', storage, whitelist: ['selectedThreadId'] };
const persistedThreadReducer = persistReducer(threadPersistConfig, threadReducer);

// Persist only previously persisted mascot appearance fields plus the custom
// GIF override added by this feature; leave existing non-persisted mascot
// fields as runtime state to avoid changing refresh behavior.
const mascotPersistConfig = {
  key: 'mascot',
  storage,
  whitelist: ['color', 'voiceId', 'customMascotGifUrl'],
};
const persistedMascotReducer = persistReducer(mascotPersistConfig, mascotReducer);

// Persona Pack v1 (issue #2345): persist the cosmetic display name + description
// per user, mirroring how mascot appearance is stored. SOUL.md lives on disk and
// is round-tripped over RPC, so it is intentionally not in this slice.
const personaPersistConfig = { key: 'persona', storage, whitelist: ['displayName', 'description'] };
const persistedPersonaReducer = persistReducer(personaPersistConfig, personaReducer);

export const store = configureStore({
  reducer: {
    socket: socketReducer,
    connectivity: connectivityReducer,
    thread: persistedThreadReducer,
    chatRuntime: chatRuntimeReducer,
    companion: companionReducer,
    agentProfiles: agentProfileReducer,
    channelConnections: persistedChannelConnectionsReducer,
    accounts: persistedAccountsReducer,
    notifications: persistedNotificationReducer,
    providerSurfaces: providerSurfacesReducer,
    coreMode: persistedCoreModeReducer,
    locale: persistedLocaleReducer,
    mascot: persistedMascotReducer,
    persona: persistedPersonaReducer,
    theme: persistedThemeReducer,
    workflows: workflowsReducer,
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
// to assert backing-state changes (see app/test/e2e/specs/*.spec.ts). Gated on
// the E2E build flag (`VITE_OPENHUMAN_E2E_RESTART_APP_AS_RELOAD`, baked by
// `app/scripts/e2e-build.sh`) so shipped production bundles do NOT expose the
// store handle — denying a same-origin attacker (compromised CDN, supply-chain
// asset, XSS) a one-call read/mutate path into full Redux state.
if (typeof window !== 'undefined' && (IS_DEV || E2E_RESTART_APP_AS_RELOAD)) {
  (window as unknown as { __OPENHUMAN_STORE__?: typeof store }).__OPENHUMAN_STORE__ = store;
}

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
