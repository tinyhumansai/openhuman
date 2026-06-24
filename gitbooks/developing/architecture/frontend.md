---
description: >-
  The React + Vite frontend (`app/src/`) - architecture, state, services,
  providers, routing, components, hooks.
icon: browsers
---

# Frontend (app/src/)

The OpenHuman desktop UI: a Vite + React 19 tree under `app/src/` (Yarn workspace `openhuman-app`). It uses Redux Toolkit with persistence for session state, talks to the backend over REST + Socket.io, and calls the Rust core sidecar via JSON-RPC (`coreRpcClient` / Tauri `core_rpc_relay`). Heavy logic lives in the core, not here.

This is one consolidated reference. Use the table of contents above (or your reader's outline) to jump between sections.

## Quick reference

| Section                                           | Covers                                        |
| ------------------------------------------------- | --------------------------------------------- |
| [Architecture](frontend.md#architecture-overview) | Provider chain, build, layout, conventions    |
| [State Management](frontend.md#state-management)  | Redux Toolkit slices, selectors, persistence  |
| [Services Layer](frontend.md#services-layer)      | `apiClient`, `socketService`, `coreRpcClient` |
| [Providers](frontend.md#providers)                | `User`, `Socket`, `AI`, `Skill` providers     |
| [Pages & Routing](frontend.md#pages-routing)      | `HashRouter`, route guards, main routes       |
| [Components](frontend.md#components)              | UI / settings component patterns              |
| [Hooks & Utilities](frontend.md#hooks-utilities)  | Shared hooks, helpers, config                 |

## Scale

| Metric                                  | Value                                                                    |
| --------------------------------------- | ------------------------------------------------------------------------ |
| TypeScript / TSX files under `app/src/` | \~285 (`find app/src -name '*.ts' -o -name '*.tsx' \| wc -l` to refresh) |
| Test runner                             | Vitest (`app/test/vitest.config.ts`)                                     |

## Directory layout

```
app/src/
├── App.tsx                 # Provider chain + HashRouter shell
├── AppRoutes.tsx           # Route table + guards
├── main.tsx                # Entry (Sentry, store, styles)
├── store/                  # Redux slices and selectors
├── providers/              # UserProvider, SocketProvider, AIProvider, SkillProvider
├── services/               # apiClient, socketService, coreRpcClient, api/*
├── lib/                    # AI loaders, MCP helpers, skills sync, etc.
├── pages/                  # Route-level screens
├── components/             # Shared UI
├── hooks/                  # App hooks
├── utils/                  # Config, Tauri helpers, routing utilities
└── assets/                 # Icons and static assets
```

## Architecture overview

### System architecture

OpenHuman’s desktop UI is a **React 19** app (`app/src/`) that:

* Uses **Redux Toolkit** with persistence for session-related state
* Connects to the backend with **REST** (`apiClient`) and **Socket.io** (`socketService`)
* Calls the **Rust core** process over HTTP via **`coreRpcClient`** / Tauri **`core_rpc_relay`** (JSON-RPC methods implemented in repo root `src/openhuman/`, exposed through `core_server`)
* Loads **AI prompts** from bundled `src/openhuman/agent/prompts` (repo root) and from Tauri **`ai_get_config`** when packaged
* Uses a **minimal MCP-style** helper layer under `lib/mcp/` (transport, validation), not a large in-repo Telegram MCP tool bundle

### Entry points

| File                    | Purpose                                                                              |
| ----------------------- | ------------------------------------------------------------------------------------ |
| `app/src/main.tsx`      | React root, Sentry boundary, store, global styles                                    |
| `app/src/App.tsx`       | Provider chain: Redux → PersistGate → User → Socket → AI → Skill → Router            |
| `app/src/AppRoutes.tsx` | `HashRouter` routes, `ProtectedRoute` / `PublicRoute`, onboarding and mnemonic gates |

### Provider chain

<!-- BEGIN GENERATED: provider-chain — source app/src/App.tsx via scripts/generate-architecture-docs.mjs; do not edit between markers (run `pnpm docs:generate`) -->

_Generated from `app/src/App.tsx` by `scripts/generate-architecture-docs.mjs`. Do not edit by hand — run `pnpm docs:generate` to refresh._

| # | Component | Role |
| --- | --- | --- |
| 1 | `Sentry.ErrorBoundary` | Crash boundary; renders ErrorFallbackScreen |
| 2 | `Provider` | Redux store; enables useAppSelector / dispatch app-wide |
| 3 | `PersistGate` | Holds UI until persisted Redux slices rehydrate |
| 4 | `ThemeProvider` | Theme tokens and dark-mode handling |
| 5 | `I18nProvider` | Localization context consumed via useT |
| 6 | `BootCheckGate` | Blocks render until the core boot snapshot resolves |
| 7 | `CoreStateProvider` | Core app snapshot: auth, session, onboarding state |
| 8 | `SocketProvider` | Core socket.io events; desktop only (mobile uses the TunnelTransport relay) |
| 9 | `ChatRuntimeProvider` | Chat runtime events, tool timeline, and approvals |
| 10 | `Router` | HashRouter navigation for all routes |
| 11 | `CommandProvider` | Command palette context |
| 12 | `ServiceBlockingGate` | Blocks the shell until required services are configured |

<!-- END GENERATED: provider-chain -->

**Why this order**

1. Redux `Provider` is outermost so `useAppSelector` / dispatch work everywhere.
2. `PersistGate` rehydrates persisted slices before children assume stable auth/session.
3. `BootCheckGate` / `CoreStateProvider` resolve the core boot snapshot (auth, onboarding) before feature providers mount.
4. `SocketProvider` (desktop only) and `ChatRuntimeProvider` depend on that core state for realtime events and approvals.
5. `Router` supplies navigation to all routes.

### Module relationships (simplified)

```
App.tsx
  ├─ Redux store + persistor
  ├─ UserProvider - user profile / workspace context
  ├─ SocketProvider - connects socketService when token present
  ├─ AIProvider - AI session / memory client coordination
  ├─ SkillProvider - skills catalog and sync
  └─ AppRoutes
       ├─ PublicRoute - e.g. Welcome on `/`
       ├─ ProtectedRoute - onboarding, home, skills, settings, …
       └─ DefaultRedirect - unauthenticated users
```

### Services layer (conceptual)

```
services/
  ├─ apiClient        → REST to a URL resolved at runtime via `services/backendUrl#getBackendUrl`
  ├─ backendUrl       → Calls `openhuman.config_resolve_api_url`; falls back to VITE_BACKEND_URL only outside Tauri
  ├─ socketService    → Socket.io; realtime + MCP-style envelopes
  └─ coreRpcClient    → HTTP to local openhuman core (JSON-RPC), used with Tauri relay
```

#### Runtime config precedence

The desktop app does not bake the core RPC URL or the API host into the bundle as a hard requirement. At runtime the app resolves them in this order (highest first):

1. **Login-screen RPC URL field**, saved via `utils/configPersistence` and restored on next launch. End users configure the sidecar address here, not by hand-editing `config.toml` or `.env` files.
2. **Tauri `core_rpc_url` command**, the port the bundled sidecar is listening on for this process.
3. **`VITE_OPENHUMAN_CORE_RPC_URL`**, build-time fallback for development.
4. The hardcoded `http://127.0.0.1:7788/rpc` default.

Once the RPC handshake succeeds, `services/backendUrl` calls `openhuman.config_resolve_api_url` to pull `api_url` (and other safe client fields) from the loaded core `Config`. `VITE_BACKEND_URL` is only used as a web fallback when the app runs outside Tauri.

Components that need the backend URL should call `useBackendUrl()` (or `getBackendUrl()` from non-React code), they must not import the static `BACKEND_URL` constant from `utils/config`, which represents the build-time value only.

### Related docs

* Rust architecture: [Architecture](../architecture.md)
* Tauri shell: [Tauri Shell](tauri-shell.md)

## State Management

The application uses Redux Toolkit with Redux-Persist for robust state management.

### Store Configuration

**File:** `store/index.ts`

```typescript
// Combines all slices with persistence
const persistConfig = {
  key: 'root',
  storage,
  whitelist: ['auth', 'telegram'], // Persisted slices
};
```

### Redux State Structure

```typescript
RootState = {
  auth: {
    token: string | null, // JWT (persisted)
    isOnboardedByUser: Record<string, boolean>, // Per-user flag (persisted)
  },
  socket: {
    byUser: Record<
      string,
      {
        // Per user ID
        status: 'connecting' | 'connected' | 'disconnected';
        socketId: string | null;
      }
    >,
  },
  user: { profile: User | null, loading: boolean, error: string | null },
  telegram: {
    byUser: Record<string, TelegramState>, // Per Telegram user (persisted)
  },
};
```

### Slices

#### Auth Slice (`store/authSlice.ts`)

Manages JWT token and per-user onboarding status.

**State:**

```typescript
interface AuthState {
  token: string | null;
  isOnboardedByUser: Record<string, boolean>;
}
```

**Actions:**

* `setToken(token: string)` - Store JWT after login
* `clearToken()` - Remove token on logout
* `setOnboarded({ userId, isOnboarded })` - Mark user as onboarded

**Selectors (`store/authSelectors.ts`):**

* `selectToken` - Get current JWT
* `selectIsOnboarded(userId)` - Check if user completed onboarding

#### Socket Slice (`store/socketSlice.ts`)

Tracks Socket.io connection status per user.

**State:**

```typescript
interface SocketState {
  byUser: Record<
    string,
    { status: 'connecting' | 'connected' | 'disconnected'; socketId: string | null }
  >;
}
```

**Actions:**

* `setSocketStatus({ userId, status })` - Update connection status
* `setSocketId({ userId, socketId })` - Store socket ID
* `clearSocketState(userId)` - Clear user's socket state

**Selectors (`store/socketSelectors.ts`):**

* `selectSocketStatus(userId)` - Get connection status
* `selectIsSocketConnected(userId)` - Boolean connected check

#### User Slice (`store/userSlice.ts`)

Stores user profile data.

**State:**

```typescript
interface UserState {
  profile: User | null;
  loading: boolean;
  error: string | null;
}
```

**Actions:**

* `setUser(user)` - Store user profile
* `setUserLoading(loading)` - Set loading state
* `setUserError(error)` - Set error state
* `clearUser()` - Clear profile on logout

#### Telegram Slice (`store/telegram/`)

Complex nested state management for Telegram integration.

**Files:**

* `index.ts` - Slice exports (actions, thunks)
* `types.ts` - Entity and state interfaces
* `reducers.ts` - Synchronous reducers
* `extraReducers.ts` - Async thunk handlers
* `thunks.ts` - Async operations

**State Structure:**

```typescript
telegram.byUser[telegramUserId] = {
  connectionStatus: "disconnected" | "connecting" | "connected" | "error",
  authStatus: "not_authenticated" | "authenticating" | "authenticated" | "error",
  currentUser: TelegramUser | null,
  sessionString: string | null,              // Stored here, NOT localStorage
  chats: Record<string, TelegramChat>,
  chatsOrder: string[],
  messages: Record<chatId, Record<msgId, TelegramMessage>>,
  threads: Record<chatId, TelegramThread[]>
}
```

**Reducers:**

* `setCurrentUser` - Store authenticated Telegram user
* `setSessionString` - Store MTProto session (for persistence)
* `setConnectionStatus` - Update connection state
* `setAuthStatus` - Update authentication state
* `addChat` / `updateChat` - Manage chat list
* `addMessage` / `updateMessage` - Manage message history
* `setThreads` - Store thread data

**Thunks (`store/telegram/thunks.ts`):**

* `initializeTelegram(userId)` - Initialize MTProto client
* `connectTelegram(userId)` - Establish Telegram connection
* `fetchChats(userId)` - Load chat list
* `fetchMessages({ userId, chatId })` - Load message history
* `disconnectTelegram(userId)` - Clean disconnect

**Selectors (`store/telegramSelectors.ts`):**

* `selectTelegramState(userId)` - Get full Telegram state
* `selectTelegramConnectionStatus(userId)` - Get connection status
* `selectTelegramAuthStatus(userId)` - Get auth status
* `selectTelegramChats(userId)` - Get chat list
* `selectTelegramMessages(userId, chatId)` - Get messages for chat

### Typed Hooks

**File:** `store/hooks.ts`

```typescript
// Use these instead of plain useDispatch/useSelector
export const useAppDispatch: () => AppDispatch = useDispatch;
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector;
```

### Persistence Configuration

#### What's Persisted

* `auth.token` - JWT for authentication
* `auth.isOnboardedByUser` - Per-user onboarding status
* `telegram.byUser` - Telegram state (sessions, chats, etc.)

#### What's NOT Persisted

* `socket` - Connection state (reconnects on app start)
* `user.loading` / `user.error` - Transient UI states
* Telegram loading/error states

#### Storage Backend

Redux-Persist uses localStorage adapter by default. This is the ONLY acceptable use of localStorage in the application.

### Usage Examples

#### Reading State

```typescript
import { useAppSelector } from '../store/hooks';

function MyComponent() {
  const token = useAppSelector(state => state.auth.token);
  const isConnected = useAppSelector(state => state.socket.byUser[userId]?.status === 'connected');
  const chats = useAppSelector(state => state.telegram.byUser[userId]?.chats);
}
```

#### Dispatching Actions

```typescript
import { clearToken, setToken } from '../store/authSlice';
import { useAppDispatch } from '../store/hooks';
import { initializeTelegram } from '../store/telegram/thunks';

function MyComponent() {
  const dispatch = useAppDispatch();

  // Sync action
  const handleLogin = (token: string) => {
    dispatch(setToken(token));
  };

  // Async thunk
  const handleConnect = async () => {
    await dispatch(initializeTelegram(userId)).unwrap();
  };
}
```

#### Using Selectors

```typescript
import { selectIsOnboarded } from '../store/authSelectors';
import { useAppSelector } from '../store/hooks';
import { selectTelegramConnectionStatus } from '../store/telegramSelectors';

function MyComponent({ userId }) {
  const isOnboarded = useAppSelector(state => selectIsOnboarded(state, userId));
  const connectionStatus = useAppSelector(state => selectTelegramConnectionStatus(state, userId));
}
```

### Best Practices

1. **Always use typed hooks** - `useAppDispatch` and `useAppSelector`
2. **Use selectors for derived state** - Memoized and testable
3. **Keep thunks in separate files** - Better organization
4. **Per-user state scoping** - Key state by user ID
5. **Avoid localStorage** - Use Redux-Persist instead

***

## Services Layer

The application uses singleton services for external communication. This prevents connection leaks and provides consistent API access.

### Service architecture

```
app/src/services/
  ├─ apiClient (HTTP REST)
  │   ├─ reads auth.token from Redux
  │   └─ calls VITE_BACKEND_URL (see utils/config.ts)
  ├─ socketService (Socket.io)
  │   ├─ web: JS client
  │   └─ Tauri: coordinates with Rust-side socket via utils/tauriSocket.ts
  ├─ coreRpcClient.ts
  │   └─ invoke('core_rpc_relay', …) → local openhuman core (JSON-RPC)
  └─ services/api/* - domain REST modules (auth, user, teams, …)
```

### API Client (`services/apiClient.ts`)

HTTP REST client for backend communication.

#### Features

* Fetch-based implementation
* Auto-injects JWT from Redux store
* Typed request/response handling
* Error handling with typed errors

#### Usage

```typescript
import apiClient from "../services/apiClient";

// GET request
const user = await apiClient.get<User>("/users/me");

// POST request
const result = await apiClient.post<LoginResponse>("/auth/login", {
  email,
  password,
});

// With custom headers
const data = await apiClient.get<Data>("/endpoint", {
  headers: { "X-Custom": "value" },
});
```

#### Configuration

Reads `VITE_BACKEND_URL` from environment or uses default:

```typescript
const BACKEND_URL =
  import.meta.env.VITE_BACKEND_URL || "https://api.example.com";
```

### API Endpoints (`services/api/`)

#### Auth API (`services/api/authApi.ts`)

Authentication-related endpoints.

```typescript
import { authApi } from "../services/api/authApi";

// Login
const { token, user } = await authApi.login(credentials);

// Token exchange (for deep link flow)
const { sessionToken, user } = await authApi.exchangeToken(loginToken);

// Logout
await authApi.logout();
```

#### User API (`services/api/userApi.ts`)

User profile endpoints.

```typescript
import { userApi } from "../services/api/userApi";

// Get current user
const user = await userApi.getCurrentUser();

// Update profile
const updated = await userApi.updateProfile({ firstName, lastName });

// Get settings
const settings = await userApi.getSettings();
```

### Socket Service (`services/socketService.ts`)

Socket.io client singleton for real-time communication.

#### Features

* Singleton pattern - single connection per app
* Auth token passed in socket `auth` object
* Transports: polling first, then WebSocket upgrade
* Auto-reconnection handling

#### API

```typescript
import socketService from "../services/socketService";

// Connect with auth token
socketService.connect(token);

// Disconnect
socketService.disconnect();

// Emit event
socketService.emit("event-name", data);

// Listen for events
socketService.on("event-name", (data) => {
  // Handle event
});

// Remove listener
socketService.off("event-name", handler);

// One-time listener
socketService.once("event-name", (data) => {
  // Handle once
});

// Get socket instance
const socket = socketService.getSocket();

// Check connection status
const isConnected = socketService.isConnected();
```

#### Connection Flow

```typescript
// In SocketProvider.tsx
useEffect(() => {
  if (token) {
    socketService.connect(token);

    socketService.on("connect", () => {
      dispatch(setSocketStatus({ userId, status: "connected" }));
      dispatch(setSocketId({ userId, socketId: socket.id }));
      // Initialize MCP server
      initMCPServer(socketService.getSocket());
    });

    socketService.on("disconnect", () => {
      dispatch(setSocketStatus({ userId, status: "disconnected" }));
    });
  }

  return () => {
    socketService.disconnect();
  };
}, [token]);
```

#### Configuration

```typescript
const socket = io(BACKEND_URL, {
  auth: { token },
  transports: ["polling", "websocket"],
  reconnection: true,
  reconnectionAttempts: 5,
  reconnectionDelay: 1000,
});
```

#### Socket event contract (Tauri)

In Tauri mode, connection and events are bridged through **`utils/tauriSocket.ts`** (`setupTauriSocketListeners`, `connectRustSocket`, etc.). See `providers/SocketProvider.tsx` for the full flow (including daemon lifecycle hooks).

### Core RPC (`services/coreRpcClient.ts`)

The desktop app runs a separate **`openhuman`** Rust binary (staged under `app/src-tauri/binaries/`). The UI calls JSON-RPC methods on that process through Tauri:

```typescript
import { callCoreRpc } from "../services/coreRpcClient";

const result = await callCoreRpc<MyType>({
  method: "some.openhuman.method",
  params: {
    /* … */
  },
  serviceManaged: false, // true if the relay should ensure the systemd/launchd-style service
});
```

Implementation: `invoke('core_rpc_relay', { request: { method, params, serviceManaged } })` → `app/src-tauri/src/commands/core_relay.rs` → HTTP client in `app/src-tauri/src/core_rpc.rs`.

### Knowledge Vault Write State

`VaultPanel` in `app/src/components/intelligence/` renders user-added knowledge vaults from the core `vault.*` controller surface. Each vault row must show the core-provided `write_state`:

- `writable` means approved markdown/wiki writes can be saved under the vault root.
- `read_only` means the folder exists but local permissions report it as read-only.
- `unavailable` means the folder is missing, no longer a directory, or cannot be inspected on this device.

The UI should not infer writability from the path string alone. It maps the core-provided `write_state_reason` code through i18n so unsupported write states are visible instead of failing silently. Approved artifact creation uses `openhuman.vault_write_markdown`; callers must pass `approved: true`, a relative `.md` or `.markdown` path, and content. The core rejects absolute paths, `..` traversal, non-markdown files, symlink escapes, and updates without `overwrite: true`.

### Service integration with providers

#### SocketProvider

`app/src/providers/SocketProvider.tsx` connects when `auth.token` is present. In **Tauri**, it prefers the Rust-backed socket path; in **web**, it uses the JS Socket.io client. See the source for logging and `useDaemonLifecycle` integration.

#### UserProvider, AIProvider, SkillProvider

These wrap user profile loading, AI/memory client coordination, and skills catalog/sync. They sit **inside** `PersistGate` and **outside** or alongside the router as shown in `App.tsx`.

### Best Practices

1. **Use singletons** - Never create multiple service instances
2. **Store sessions in Redux** - Not localStorage
3. **Clean up on unmount** - Disconnect in useEffect cleanup
4. **Handle errors gracefully** - Retry for transient failures
5. **Pass auth via proper channels** - Socket auth object, not query string

***

## Providers

React context providers manage service lifecycle and provide shared state.

### Provider chain

The providers wrap the application in a specific order (`app/src/App.tsx`):

```tsx
<Sentry.ErrorBoundary>
  <Provider store={store}>
    <PersistGate persistor={persistor} onBeforeLift={...}>
      <UserProvider>
        <SocketProvider>
          <AIProvider>
            <SkillProvider>
              <Router>
                <AppRoutes />
              </Router>
            </SkillProvider>
          </AIProvider>
        </SocketProvider>
      </UserProvider>
    </PersistGate>
  </Provider>
</Sentry.ErrorBoundary>
```

(`Router` is `HashRouter` from `react-router-dom`.)

**Order matters because:**

1. Redux is outermost for store access.
2. `PersistGate` rehydrates persisted slices before children rely on auth.
3. `SocketProvider` uses the JWT from the store.
4. `AIProvider` / `SkillProvider` depend on socket and store-backed features.
5. The router supplies navigation to all routes.

### SocketProvider (`app/src/providers/SocketProvider.tsx`)

Manages realtime connectivity: **web** uses the JS Socket.io client; **Tauri** bridges to the Rust socket via `utils/tauriSocket.ts` and reports status back to Redux.

#### Responsibilities

* Connect when `auth.token` is available; disconnect when cleared
* In Tauri: install listeners once, connect Rust socket, coordinate daemon lifecycle (`useDaemonLifecycle`)
* Update Redux socket slice / connection status

#### Implementation

See **`app/src/providers/SocketProvider.tsx`**. The file branches on **`isTauri()`**: web mode uses `socketService` directly; Tauri sets up `tauriSocket` listeners and `connectRustSocket` / `disconnectRustSocket`. Do not treat the pseudocode below as the live implementation.

#### Usage

```typescript
import { useSocket } from '../providers/SocketProvider';

function MyComponent() {
  const { socket, isConnected, emit, on, off } = useSocket();

  useEffect(() => {
    const handler = (data) => console.log('Received:', data);
    on('event-name', handler);
    return () => off('event-name', handler);
  }, [on, off]);

  const sendMessage = () => {
    emit('send-message', { text: 'Hello!' });
  };

  return (
    <div>
      <span>Status: {isConnected ? 'Connected' : 'Disconnected'}</span>
      <button onClick={sendMessage}>Send</button>
    </div>
  );
}
```

### AIProvider (`app/src/providers/AIProvider.tsx`)

Initializes **memory**, **sessions**, **tool registry** (including memory + web-search tools), **entity manager**, **LLM / embedding providers**, and **constitution** loading. Exposes `useAI()` for children. Heavy logic lives under `app/src/lib/ai/`.

### SkillProvider (`app/src/providers/SkillProvider.tsx`)

On mount (when authenticated), discovers skills from the **QuickJS** skills engine via Tauri helpers (`runtimeDiscoverSkills`), syncs manifests into Redux, listens for skill-related Tauri events, and can auto-start configured skills in development.

### UserProvider (`providers/UserProvider.tsx`)

Minimal user context provider (most user state is in Redux).

#### Responsibilities

* Legacy user context for compatibility
* May be deprecated in favor of Redux

#### Implementation

```typescript
interface UserContextValue {
  user: User | null;
  loading: boolean;
}

export function UserProvider({ children }) {
  const user = useAppSelector((state) => state.user.profile);
  const loading = useAppSelector((state) => state.user.loading);

  return (
    <UserContext.Provider value={{ user, loading }}>
      {children}
    </UserContext.Provider>
  );
}
```

#### Usage

```typescript
import { useUserContext } from '../providers/UserProvider';

function Header() {
  const { user, loading } = useUserContext();

  if (loading) return <Skeleton />;
  if (!user) return null;

  return <span>Welcome, {user.firstName}</span>;
}
```

### Provider Patterns

#### Effect-Based Lifecycle

Providers use `useEffect` to manage service lifecycle:

```typescript
useEffect(() => {
  // Setup on mount or dependency change
  service.connect();

  // Cleanup on unmount or dependency change
  return () => {
    service.disconnect();
  };
}, [dependencies]);
```

#### Redux Integration

Providers read from and dispatch to Redux:

```typescript
// Read state
const token = useAppSelector((state) => state.auth.token);

// Dispatch actions
const dispatch = useAppDispatch();
dispatch(setStatus({ userId, status: "connected" }));
```

#### Parallel initialization

`SkillProvider` and `AIProvider` may kick off several async tasks on mount (skill discovery, memory init, constitution load). Prefer reading the source for ordering guarantees rather than assuming parallel `Promise.all` everywhere.

#### Session Restoration

Providers restore persisted state on mount:

```typescript
useEffect(() => {
  if (persistedSession) {
    service.restoreSession(persistedSession);
  }
}, [persistedSession]);
```

### Context vs Redux

| Use Context For                    | Use Redux For                      |
| ---------------------------------- | ---------------------------------- |
| Service instances (socket, client) | Serializable state (status, data)  |
| Methods (emit, on, off)            | Persisted state (sessions, tokens) |
| Derived values                     | Complex state logic                |

Example:

* `SocketContext` provides `socket` instance and `emit` method
* Redux stores `socketStatus` and `socketId`

### Testing Providers

#### Mock Provider for Tests

```typescript
// test-utils.tsx
const mockSocketContext: SocketContextValue = {
  socket: null,
  isConnected: true,
  emit: jest.fn(),
  on: jest.fn(),
  off: jest.fn()
};

export function TestProviders({ children }) {
  return (
    <Provider store={testStore}>
      <SocketContext.Provider value={mockSocketContext}>
        {children}
      </SocketContext.Provider>
    </Provider>
  );
}
```

#### Testing Provider Effects

```typescript
test('SocketProvider connects when token is available', () => {
  const store = createTestStore({ auth: { token: 'test-token' } });

  render(
    <Provider store={store}>
      <SocketProvider>
        <TestComponent />
      </SocketProvider>
    </Provider>
  );

  expect(socketService.connect).toHaveBeenCalledWith('test-token');
});
```

***

## Human Mascot Surface

The Human page (`app/src/features/human/HumanPage.tsx`) renders the main
`YellowMascot` beside the conversation sidebar. The mascot face still comes
from `useHumanMascot`, which subscribes to chat lifecycle events for thinking,
speaking, acknowledgement, and error states.

Sub-agent delegation is visualized by `SubMascotLayer`. It does not introduce a
new socket protocol. Instead, it reads the selected or active thread's
`chatRuntime.toolTimelineByThread` entries that `ChatRuntimeProvider` already
builds from `subagent_spawned`, `subagent_completed`, `subagent_failed`,
`subagent_iteration_start`, `subagent_tool_call`, and `subagent_tool_result`.

Lifecycle mapping:

| Runtime timeline state | Sub-mascot state |
| ---------------------- | ---------------- |
| `running`              | Small colored mascot in a thinking face with a short activity bubble |
| `success`              | Same mascot resolves to a happy face and completion bubble |
| `error`                | Same mascot resolves to a concerned face and failure bubble |

Activity bubble text is intentionally compact: current child tool call, child
iteration, the delegation prompt excerpt, or final status. The thread timeline
remains the authoritative detailed view; sub-mascots are only the glanceable
orchestration layer around the main mascot.

***

## Pages & Routing

The application uses HashRouter with protected and public route guards.

### Route structure

Defined in **`app/src/AppRoutes.tsx`** (HashRouter). Approximate map:

```
/                  → Welcome (public wrapper)
/onboarding        → Onboarding (auth, onboarding not complete)
/mnemonic          → Mnemonic / encryption setup (auth)
/home              → Home (auth + onboarding + encryption key)
/intelligence      → Intelligence (auth)
/skills            → Skills (auth)
/conversations     → Conversations (auth)
/invites           → Invites (auth)
/agents            → Agents (auth)
/settings/*        → Settings (auth)
*                  → DefaultRedirect
```

There is **no** top-level `/login` route in `AppRoutes`; authentication flows are handled via welcome/onboarding and backend redirects.

### Route Configuration (`AppRoutes.tsx`)

```typescript
export function AppRoutes() {
  return (
    <>
      <Routes>
        {/* Public routes - redirect if authenticated */}
        <Route element={<PublicRoute />}>
          <Route path="/" element={<Welcome />} />
          <Route path="/login" element={<Login />} />
        </Route>

        {/* Protected routes - require authentication */}
        <Route element={<ProtectedRoute />}>
          <Route path="/onboarding/*" element={<Onboarding />} />
        </Route>

        {/* Protected + onboarded routes */}
        <Route element={<ProtectedRoute requireOnboarded />}>
          <Route path="/home" element={<Home />} />
        </Route>

        {/* Fallback redirect */}
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>

      {/* Settings modal overlay - renders on top of routes */}
      <SettingsModal />
    </>
  );
}
```

### Route Guards

#### PublicRoute (`components/PublicRoute.tsx`)

Redirects authenticated users away from public pages.

```typescript
export function PublicRoute() {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (token) {
    // Authenticated - redirect to appropriate page
    return <Navigate to={isOnboarded ? "/home" : "/onboarding"} replace />;
  }

  return <Outlet />;
}
```

#### ProtectedRoute (`components/ProtectedRoute.tsx`)

Enforces authentication and optionally onboarding status.

```typescript
interface ProtectedRouteProps {
  requireOnboarded?: boolean;
}

export function ProtectedRoute({ requireOnboarded = false }) {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (!token) {
    return <Navigate to="/login" replace />;
  }

  if (requireOnboarded && !isOnboarded) {
    return <Navigate to="/onboarding" replace />;
  }

  return <Outlet />;
}
```

#### DefaultRedirect (`components/DefaultRedirect.tsx`)

Fallback route that redirects based on auth state.

```typescript
export function DefaultRedirect() {
  const token = useAppSelector((state) => state.auth.token);
  const isOnboarded = useAppSelector((state) =>
    selectIsOnboarded(state, userId),
  );

  if (!token) {
    return <Navigate to="/" replace />;
  }

  if (!isOnboarded) {
    return <Navigate to="/onboarding" replace />;
  }

  return <Navigate to="/home" replace />;
}
```

### Pages

#### Welcome Page (`pages/Welcome.tsx`)

Landing page for unauthenticated users.

**Features:**

* App introduction and branding
* CTA to login/signup
* Public route (redirects if authenticated)

#### Login Page (`pages/Login.tsx`)

Authentication page.

**Features:**

* Telegram OAuth button
* Opens `/auth/telegram?platform=desktop` in browser
* Handles deep link callback

```typescript
export function Login() {
  const handleTelegramLogin = () => {
    // Opens Telegram OAuth in system browser
    openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`);
  };

  return (
    <div className="login-page">
      <TelegramLoginButton onClick={handleTelegramLogin} />
    </div>
  );
}
```

#### Home Page (`pages/Home.tsx`)

Main dashboard after authentication.

**Features:**

* Protected route (requires auth + onboarded)
* Connection status indicators
* Navigation to settings modal
* Future: Chat list, messages, etc.

```typescript
export function Home() {
  const navigate = useNavigate();
  const user = useAppSelector((state) => state.user.profile);
  const telegramStatus = useAppSelector((state) =>
    selectTelegramConnectionStatus(state, user?.id),
  );

  return (
    <div className="home-page">
      <header>
        <h1>Welcome, {user?.firstName}</h1>
        <button onClick={() => navigate("/settings")}>Settings</button>
      </header>

      <TelegramConnectionIndicator status={telegramStatus} />
      <ConnectionIndicator />

      {/* Main content */}
    </div>
  );
}
```

### Onboarding Flow (`pages/onboarding/`)

Multi-step onboarding process.

#### Structure

```
pages/onboarding/
├── Onboarding.tsx           # Flow controller
└── steps/
    ├── GetStartedStep.tsx   # Welcome
    ├── PrivacyStep.tsx      # Privacy policy
    ├── AnalyticsStep.tsx    # Analytics opt-in
    ├── ConnectStep.tsx      # Telegram connection
    └── FeaturesStep.tsx     # Features overview
```

#### Onboarding Controller (`Onboarding.tsx`)

```typescript
const STEPS = [
  { id: "get-started", component: GetStartedStep },
  { id: "privacy", component: PrivacyStep },
  { id: "analytics", component: AnalyticsStep },
  { id: "connect", component: ConnectStep },
  { id: "features", component: FeaturesStep },
];

export function Onboarding() {
  const [currentStep, setCurrentStep] = useState(0);
  const dispatch = useAppDispatch();
  const navigate = useNavigate();

  const handleNext = () => {
    if (currentStep < STEPS.length - 1) {
      setCurrentStep(currentStep + 1);
    } else {
      // Complete onboarding
      dispatch(setOnboarded({ userId, isOnboarded: true }));
      navigate("/home");
    }
  };

  const handleBack = () => {
    if (currentStep > 0) {
      setCurrentStep(currentStep - 1);
    }
  };

  const StepComponent = STEPS[currentStep].component;

  return (
    <div className="onboarding">
      <ProgressIndicator current={currentStep} total={STEPS.length} />
      <StepComponent onNext={handleNext} onBack={handleBack} />
    </div>
  );
}
```

#### Step Components

Each step receives `onNext` and `onBack` callbacks:

```typescript
interface StepProps {
  onNext: () => void;
  onBack: () => void;
}

export function ConnectStep({ onNext, onBack }: StepProps) {
  const [showModal, setShowModal] = useState(false);
  const telegramStatus = useAppSelector(/* ... */);

  return (
    <div className="step">
      <h2>Connect Your Accounts</h2>

      {connectOptions.map((option) => (
        <ConnectionOption
          key={option.id}
          {...option}
          onClick={() => option.id === "telegram" && setShowModal(true)}
        />
      ))}

      <TelegramConnectionModal
        isOpen={showModal}
        onClose={() => setShowModal(false)}
      />

      <div className="actions">
        <button onClick={onBack}>Back</button>
        <button onClick={onNext}>Continue</button>
      </div>
    </div>
  );
}
```

### Settings Modal Routing

The settings modal overlays existing content using URL-based routing.

#### Modal Detection

```typescript
// In SettingsModal.tsx
const location = useLocation();
const isOpen = location.pathname.startsWith("/settings");
```

#### Sub-Routes

```
/settings              → SettingsHome (main menu)
/settings/connections  → ConnectionsPanel
/settings/messaging    → MessagingPanel (future)
/settings/privacy      → PrivacyPanel (future)
/settings/profile      → ProfilePanel (future)
/settings/advanced     → AdvancedPanel (future)
/settings/billing      → BillingPanel (future)
```

#### Navigation

```typescript
import { useSettingsNavigation } from "./hooks/useSettingsNavigation";

function SettingsHome() {
  const { navigateTo, closeModal } = useSettingsNavigation();

  return (
    <div>
      <SettingsMenuItem
        label="Connections"
        onClick={() => navigateTo("connections")}
      />
      <button onClick={closeModal}>Close</button>
    </div>
  );
}
```

### HashRouter vs BrowserRouter

The app uses HashRouter for desktop compatibility:

```typescript
// App.tsx
import { HashRouter } from "react-router-dom";

// URLs look like: app://localhost/#/home
// Instead of: app://localhost/home
```

**Why HashRouter:**

1. Tauri deep links work with hash-based URLs
2. No server configuration needed
3. Works with file:// protocol
4. Prevents 404 on direct URL access

### Deep Link Handling

Deep links are handled before routing:

```typescript
// main.tsx
import("./utils/desktopDeepLinkListener").then((m) => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

The listener intercepts `openhuman://auth?token=...` and:

1. Exchanges token via Rust command
2. Stores session in Redux
3. Navigates to `/onboarding` or `/home`

### Navigation Patterns

#### Programmatic Navigation

```typescript
import { useNavigate } from "react-router-dom";

const navigate = useNavigate();

// Navigate to route
navigate("/home");

// Replace history entry
navigate("/login", { replace: true });

// Go back
navigate(-1);
```

#### Link Component

```typescript
import { Link } from "react-router-dom";

<Link to="/settings">Settings</Link>;
```

#### State Transfer

```typescript
// Pass state to route
navigate("/details", { state: { itemId: 123 } });

// Receive state
const location = useLocation();
const { itemId } = location.state;
```

***

## Components

Reusable React components organized by feature.

### Component Structure

```
components/
├── Route Guards
│   ├── ProtectedRoute.tsx
│   ├── PublicRoute.tsx
│   └── DefaultRedirect.tsx
│
├── Authentication
│   └── TelegramLoginButton.tsx
│
├── Connection Status
│   ├── ConnectionIndicator.tsx
│   ├── TelegramConnectionIndicator.tsx
│   ├── TelegramConnectionModal.tsx
│   └── GmailConnectionIndicator.tsx
│
├── Onboarding
│   ├── ProgressIndicator.tsx
│   └── LottieAnimation.tsx
│
├── Settings Modal (16 files)
│   ├── SettingsModal.tsx
│   ├── SettingsLayout.tsx
│   ├── SettingsHome.tsx
│   ├── panels/
│   ├── components/
│   └── hooks/
│
└── Development
    └── DesignSystemShowcase.tsx
```

### Route Guard Components

#### ProtectedRoute

Requires authentication and optionally onboarding.

```typescript
interface ProtectedRouteProps {
  requireOnboarded?: boolean;
}

// Usage in AppRoutes.tsx
<Route element={<ProtectedRoute />}>
  <Route path="/onboarding/*" element={<Onboarding />} />
</Route>

<Route element={<ProtectedRoute requireOnboarded />}>
  <Route path="/home" element={<Home />} />
</Route>
```

#### PublicRoute

Redirects authenticated users away.

```typescript
// Usage in AppRoutes.tsx
<Route element={<PublicRoute />}>
  <Route path="/" element={<Welcome />} />
  <Route path="/login" element={<Login />} />
</Route>
```

#### DefaultRedirect

Fallback that routes based on auth state.

```typescript
// Redirects to:
// - "/" if not authenticated
// - "/onboarding" if authenticated but not onboarded
// - "/home" if authenticated and onboarded
```

### Authentication Components

#### TelegramLoginButton

OAuth login button for Telegram.

```typescript
interface TelegramLoginButtonProps {
  onClick: () => void;
  disabled?: boolean;
}

// Usage
<TelegramLoginButton
  onClick={() => openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`)}
/>
```

### Connection Status Components

#### ConnectionIndicator

Generic connection status badge.

```typescript
interface ConnectionIndicatorProps {
  status: 'connected' | 'connecting' | 'disconnected' | 'error';
  label?: string;
}

<ConnectionIndicator status="connected" label="Socket" />
```

#### TelegramConnectionIndicator

Telegram-specific status display.

```typescript
interface TelegramConnectionIndicatorProps {
  status: 'connected' | 'connecting' | 'disconnected' | 'error';
}

// Usage with Redux state
const telegramStatus = useAppSelector((state) =>
  selectTelegramConnectionStatus(state, userId)
);

<TelegramConnectionIndicator status={telegramStatus} />
```

#### TelegramConnectionModal

Modal for setting up Telegram connection.

```typescript
interface TelegramConnectionModalProps {
  isOpen: boolean;
  onClose: () => void;
}

// Usage in onboarding/settings
const [showModal, setShowModal] = useState(false);

<TelegramConnectionModal
  isOpen={showModal}
  onClose={() => setShowModal(false)}
/>
```

**Features:**

* QR code login flow
* Phone number login flow
* Connection status display
* Error handling

#### GmailConnectionIndicator

Gmail status badge (future integration).

```typescript
<GmailConnectionIndicator status="coming-soon" />
```

### Onboarding Components

#### ProgressIndicator

Visual progress through onboarding steps.

```typescript
interface ProgressIndicatorProps {
  current: number;
  total: number;
}

<ProgressIndicator current={2} total={5} />
```

#### LottieAnimation

Lottie animation player for onboarding.

```typescript
interface LottieAnimationProps {
  animationData: object;
  loop?: boolean;
  autoplay?: boolean;
  className?: string;
}

import welcomeAnimation from '../assets/animations/welcome.json';

<LottieAnimation
  animationData={welcomeAnimation}
  loop={true}
  autoplay={true}
/>
```

### Settings Modal System

Complete modal system with URL-based routing.

#### File Structure

```
components/settings/
├── SettingsModal.tsx          # Route-based container
├── SettingsLayout.tsx         # Portal + backdrop wrapper
├── SettingsHome.tsx           # Main menu with profile
├── panels/
│   ├── ConnectionsPanel.tsx   # Connection management
│   ├── MessagingPanel.tsx     # (Future)
│   ├── PrivacyPanel.tsx       # (Future)
│   ├── ProfilePanel.tsx       # (Future)
│   ├── AdvancedPanel.tsx      # (Future)
│   └── BillingPanel.tsx       # (Future)
├── components/
│   ├── SettingsHeader.tsx     # User profile section
│   ├── SettingsMenuItem.tsx   # Menu item component
│   ├── SettingsBackButton.tsx # Back navigation
│   └── SettingsPanelLayout.tsx# Panel wrapper
└── hooks/
    ├── useSettingsNavigation.ts # URL routing
    └── useSettingsAnimation.ts  # Animation state
```

#### SettingsModal

Main container that renders based on URL.

```typescript
export function SettingsModal() {
  const location = useLocation();
  const isOpen = location.pathname.startsWith('/settings');

  if (!isOpen) return null;

  return (
    <SettingsLayout>
      {/* Route to appropriate panel */}
      {location.pathname === '/settings' && <SettingsHome />}
      {location.pathname === '/settings/connections' && <ConnectionsPanel />}
      {/* ... more panels */}
    </SettingsLayout>
  );
}
```

#### SettingsLayout

Portal-based modal wrapper.

```typescript
export function SettingsLayout({ children }) {
  const { closeModal } = useSettingsNavigation();

  return createPortal(
    <div className="fixed inset-0 z-50">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={closeModal}
      />

      {/* Modal */}
      <div className="absolute inset-4 flex items-center justify-center">
        <div className="bg-white rounded-2xl w-full max-w-[520px] shadow-xl">
          {children}
        </div>
      </div>
    </div>,
    document.body
  );
}
```

#### SettingsHome

Main menu with user profile.

```typescript
export function SettingsHome() {
  const { navigateTo, closeModal } = useSettingsNavigation();
  const user = useAppSelector((state) => state.user.profile);

  const menuItems = [
    { id: 'connections', label: 'Connections', icon: LinkIcon },
    { id: 'messaging', label: 'Messaging', icon: MessageIcon },
    { id: 'privacy', label: 'Privacy', icon: ShieldIcon },
    // ... more items
  ];

  return (
    <div>
      <SettingsHeader user={user} onClose={closeModal} />

      {menuItems.map((item) => (
        <SettingsMenuItem
          key={item.id}
          {...item}
          onClick={() => navigateTo(item.id)}
        />
      ))}
    </div>
  );
}
```

#### ConnectionsPanel

Connection management interface.

```typescript
export function ConnectionsPanel() {
  const { navigateBack } = useSettingsNavigation();
  const [telegramModalOpen, setTelegramModalOpen] = useState(false);

  const telegramStatus = useAppSelector((state) =>
    selectTelegramConnectionStatus(state, userId)
  );

  // Reuses connectOptions from onboarding
  const connections = connectOptions.map((opt) => ({
    ...opt,
    status: opt.id === 'telegram' ? telegramStatus : 'coming-soon'
  }));

  return (
    <SettingsPanelLayout title="Connections" onBack={navigateBack}>
      {connections.map((conn) => (
        <ConnectionItem
          key={conn.id}
          {...conn}
          onConnect={() => conn.id === 'telegram' && setTelegramModalOpen(true)}
        />
      ))}

      <TelegramConnectionModal
        isOpen={telegramModalOpen}
        onClose={() => setTelegramModalOpen(false)}
      />
    </SettingsPanelLayout>
  );
}
```

#### Settings Hooks

**useSettingsNavigation**

URL-based navigation for settings modal.

```typescript
interface UseSettingsNavigationReturn {
  currentRoute: string;
  navigateTo: (panel: string) => void;
  navigateBack: () => void;
  closeModal: () => void;
}

const { navigateTo, navigateBack, closeModal } = useSettingsNavigation();

// Navigate to panel
navigateTo('connections'); // → /settings/connections

// Go back
navigateBack(); // → /settings

// Close modal
closeModal(); // → previous non-settings route
```

**useSettingsAnimation**

Animation state management.

```typescript
interface UseSettingsAnimationReturn {
  isEntering: boolean;
  isExiting: boolean;
  animationClass: string;
}

const { animationClass } = useSettingsAnimation();

<div className={`modal ${animationClass}`}>
  {/* Content */}
</div>
```

#### Settings Components

**SettingsHeader**

User profile section at top of settings.

```typescript
interface SettingsHeaderProps {
  user: User | null;
  onClose: () => void;
}

<SettingsHeader user={user} onClose={handleClose} />
```

**SettingsMenuItem**

Individual menu item with icon and chevron.

```typescript
interface SettingsMenuItemProps {
  label: string;
  icon: React.ComponentType;
  onClick: () => void;
  badge?: string;
  disabled?: boolean;
}

<SettingsMenuItem
  label="Connections"
  icon={LinkIcon}
  onClick={() => navigateTo('connections')}
  badge="2"
/>
```

**SettingsBackButton**

Back navigation button.

```typescript
interface SettingsBackButtonProps {
  onClick: () => void;
}

<SettingsBackButton onClick={navigateBack} />
```

**SettingsPanelLayout**

Wrapper for settings panels.

```typescript
interface SettingsPanelLayoutProps {
  title: string;
  onBack: () => void;
  children: React.ReactNode;
}

<SettingsPanelLayout title="Connections" onBack={navigateBack}>
  {/* Panel content */}
</SettingsPanelLayout>
```

### Component Patterns

#### Reusing Connection Options

The `connectOptions` array is shared between onboarding and settings:

```typescript
// Defined in ConnectStep.tsx, imported elsewhere
export const connectOptions = [
  {
    id: 'telegram',
    label: 'Telegram',
    icon: TelegramIcon,
    description: 'Connect your Telegram account',
  },
  {
    id: 'gmail',
    label: 'Gmail',
    icon: GmailIcon,
    description: 'Connect your Gmail account',
    comingSoon: true,
  },
];
```

#### Modal via Portal

Settings modal uses `createPortal` to render outside the component tree:

```typescript
return createPortal(
  <div className="modal-container">
    {/* Modal content */}
  </div>,
  document.body
);
```

#### Controlled vs Uncontrolled

Connection modals are controlled components:

```typescript
// Parent controls open state
const [isOpen, setIsOpen] = useState(false);

<TelegramConnectionModal
  isOpen={isOpen}
  onClose={() => setIsOpen(false)}
/>
```

***

## Hooks & Utilities

Custom React hooks and utility functions.

### Custom Hooks

#### useSocket (`hooks/useSocket.ts`)

Access Socket.io functionality from any component.

```typescript
interface UseSocketReturn {
  socket: Socket | null;
  isConnected: boolean;
  emit: (event: string, data: unknown) => void;
  on: (event: string, handler: Function) => void;
  off: (event: string, handler: Function) => void;
  once: (event: string, handler: Function) => void;
}

function useSocket(): UseSocketReturn;
```

**Usage:**

```typescript
import { useSocket } from "../hooks/useSocket";

function ChatInput() {
  const { emit, isConnected } = useSocket();

  const sendMessage = (text: string) => {
    if (isConnected) {
      emit("chat:message", { text });
    }
  };

  return (
    <input
      disabled={!isConnected}
      onKeyDown={(e) => e.key === "Enter" && sendMessage(e.target.value)}
    />
  );
}
```

**With event listeners:**

```typescript
function Notifications() {
  const { on, off } = useSocket();
  const [notifications, setNotifications] = useState([]);

  useEffect(() => {
    const handler = (notification) => {
      setNotifications((prev) => [...prev, notification]);
    };

    on("notification", handler);
    return () => off("notification", handler);
  }, [on, off]);

  return <NotificationList items={notifications} />;
}
```

#### useUser (`hooks/useUser.ts`)

Access user profile data and loading state.

```typescript
interface UseUserReturn {
  user: User | null;
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
}

function useUser(): UseUserReturn;
```

**Usage:**

```typescript
import { useUser } from "../hooks/useUser";

function ProfileHeader() {
  const { user, loading, error, refetch } = useUser();

  if (loading) return <Skeleton />;
  if (error) return <Error message={error} onRetry={refetch} />;
  if (!user) return null;

  return (
    <div className="profile">
      <Avatar src={user.avatar} />
      <span>
        {user.firstName} {user.lastName}
      </span>
    </div>
  );
}
```

#### Settings Modal Hooks

**useSettingsNavigation (`components/settings/hooks/useSettingsNavigation.ts`)**

URL-based navigation for settings modal.

```typescript
interface UseSettingsNavigationReturn {
  currentRoute: string; // Current settings path
  navigateTo: (panel: string) => void; // Navigate to panel
  navigateBack: () => void; // Go back one level
  closeModal: () => void; // Close settings entirely
}

function useSettingsNavigation(): UseSettingsNavigationReturn;
```

**Usage:**

```typescript
import { useSettingsNavigation } from "./hooks/useSettingsNavigation";

function SettingsMenu() {
  const { navigateTo, closeModal } = useSettingsNavigation();

  return (
    <nav>
      <button onClick={() => navigateTo("connections")}>Connections</button>
      <button onClick={() => navigateTo("privacy")}>Privacy</button>
      <button onClick={closeModal}>Close</button>
    </nav>
  );
}
```

**useSettingsAnimation (`components/settings/hooks/useSettingsAnimation.ts`)**

Animation state management for settings modal.

```typescript
interface UseSettingsAnimationReturn {
  isEntering: boolean; // Modal is animating in
  isExiting: boolean; // Modal is animating out
  animationClass: string; // CSS class for current state
}

function useSettingsAnimation(): UseSettingsAnimationReturn;
```

**Usage:**

```typescript
import { useSettingsAnimation } from "./hooks/useSettingsAnimation";

function SettingsModal() {
  const { animationClass, isExiting } = useSettingsAnimation();

  return <div className={`modal ${animationClass}`}>{/* Content */}</div>;
}
```

### Utilities

#### Configuration (`utils/config.ts`)

Build-time environment variable access. These constants only carry the value that was baked into the bundle, for the **runtime** URL the app actually talks to, see `services/backendUrl` and `hooks/useBackendUrl` below.

```typescript
// Build-time fallback only (used outside Tauri).
export const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || 'https://api.example.com';

// Debug mode
export const DEBUG = import.meta.env.VITE_DEBUG === 'true';
```

**Usage (build-time only, feature flags, debug toggles, …):**

```typescript
import { DEBUG } from '../utils/config';

if (DEBUG) {
  console.log('debug enabled');
}
```

> **Do not** import `BACKEND_URL` directly to make API calls. Resolve the URL at runtime so the core sidecar's `api_url` (set on the login screen via `openhuman.config_resolve_api_url`) takes effect:
>
> ```typescript
> // React components
> import { useBackendUrl } from '../hooks/useBackendUrl';
> const backendUrl = useBackendUrl();
>
> // Non-React code
> import { getBackendUrl } from '../services/backendUrl';
> const backendUrl = await getBackendUrl();
> ```

#### Deep Link (`utils/deeplink.ts`)

Build deep link URLs for authentication handoff.

```typescript
// Build auth deep link
function buildAuthDeepLink(token: string): string;

// Parse deep link URL
function parseDeepLink(url: string): { path: string; params: URLSearchParams };
```

**Usage:**

```typescript
import { buildAuthDeepLink } from '../utils/deeplink';

// Build URL for browser redirect
const deepLink = buildAuthDeepLink(loginToken);
// → "openhuman://auth?token=abc123"

// In web frontend after auth:
window.location.href = deepLink;
```

#### Desktop Deep Link Listener (`utils/desktopDeepLinkListener.ts`)

Handle incoming deep links in desktop app.

```typescript
// Setup listener for deep link events
async function setupDesktopDeepLinkListener(): Promise<void>;
```

**Called in main.tsx:**

```typescript
// Lazy import to ensure Tauri IPC is ready
import('./utils/desktopDeepLinkListener').then(m => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

**What it does:**

1. Listens for `onOpenUrl` events from Tauri deep-link plugin
2. Parses `openhuman://auth?token=...` URLs
3. Calls Rust `exchange_token` command (bypasses CORS)
4. Stores session in Redux
5. Navigates to `/onboarding` or `/home`

**Loop prevention:**

```typescript
// Set flag before navigation to prevent reprocessing
localStorage.setItem('deepLinkHandled', 'true');
window.location.replace('/');

// On next load, clear flag
if (localStorage.getItem('deepLinkHandled') === 'true') {
  localStorage.removeItem('deepLinkHandled');
  return; // Don't process again
}
```

#### URL Opener (`utils/openUrl.ts`)

Cross-platform URL opening.

```typescript
// Open URL in system browser
async function openUrl(url: string): Promise<void>;
```

**Usage:**

```typescript
import { openUrl } from '../utils/openUrl';

// Opens in system browser (not in-app WebView)
await openUrl('https://telegram.org/auth');
```

**Implementation:**

```typescript
export async function openUrl(url: string): Promise<void> {
  try {
    // Try Tauri opener plugin first
    const { open } = await import('@tauri-apps/plugin-opener');
    await open(url);
  } catch {
    // Fallback to browser API
    window.open(url, '_blank');
  }
}
```

### Polyfills (`polyfills.ts`)

Node.js polyfills for browser environment.

The `telegram` npm package requires Node.js APIs. These are polyfilled:

```typescript
// polyfills.ts
import { Buffer } from 'buffer';
import process from 'process';
import util from 'util';

window.Buffer = Buffer;
window.process = process;
window.util = util;
```

**Imported at app entry:**

```typescript
// main.tsx
import './polyfills';

// ... rest of app
```

**Vite configuration:**

```typescript
// vite.config.ts
export default defineConfig({
  resolve: { alias: { buffer: 'buffer', process: 'process/browser', util: 'util' } },
  define: { 'process.env': {}, global: 'globalThis' },
});
```

### Types

#### API Types (`types/api.ts`)

```typescript
// API response wrapper
interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

// API error
interface ApiError {
  code: string;
  message: string;
  details?: unknown;
}

// User interface
interface User {
  id: string;
  firstName: string;
  lastName?: string;
  username?: string;
  email?: string;
  avatar?: string;
  telegramId?: string;
  subscription?: SubscriptionInfo;
  usage?: UsageInfo;
  createdAt: string;
  updatedAt: string;
}
```

#### Onboarding Types (`types/onboarding.ts`)

```typescript
// Onboarding step definition
interface OnboardingStep {
  id: string;
  title: string;
  component: React.ComponentType<StepProps>;
}

// Step component props
interface StepProps {
  onNext: () => void;
  onBack: () => void;
}

// Connection option
interface ConnectionOption {
  id: string;
  label: string;
  icon: React.ComponentType;
  description: string;
  comingSoon?: boolean;
}
```

### Static Data

#### Countries (`data/countries.ts`)

Country list for phone number input.

```typescript
interface Country {
  code: string; // "US"
  name: string; // "United States"
  dialCode: string; // "+1"
  flag: string; // "🇺🇸"
}

export const countries: Country[];
```

**Usage:**

```typescript
import { countries } from "../data/countries";

function PhoneInput() {
  const [country, setCountry] = useState(countries[0]);

  return (
    <div>
      <select
        value={country.code}
        onChange={(e) =>
          setCountry(countries.find((c) => c.code === e.target.value))
        }
      >
        {countries.map((c) => (
          <option key={c.code} value={c.code}>
            {c.flag} {c.name} ({c.dialCode})
          </option>
        ))}
      </select>
      <input placeholder="Phone number" />
    </div>
  );
}
```

### Best Practices

#### Hook Dependencies

Always include dependencies in useEffect:

```typescript
// Good
useEffect(() => {
  on('event', handler);
  return () => off('event', handler);
}, [on, off, handler]);

// Bad - missing dependencies
useEffect(() => {
  on('event', handler);
  return () => off('event', handler);
}, []);
```

#### Cleanup Functions

Always clean up subscriptions:

```typescript
useEffect(() => {
  const subscription = subscribe();
  return () => subscription.unsubscribe();
}, []);
```

#### Error Boundaries

Wrap utility calls in try-catch:

```typescript
try {
  await openUrl(url);
} catch (error) {
  console.error('Failed to open URL:', error);
  // Fallback behavior
}
```

#### Type Safety

Use TypeScript generics for API calls:

```typescript
const user = await apiClient.get<User>('/users/me');
// user is typed as User
```

***
