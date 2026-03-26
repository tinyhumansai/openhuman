# Architecture Overview

## System Architecture

The Outsourced platform is built on a layered architecture supporting:

- Redux-based state management with persistence
- Socket.io real-time communication
- Telegram MTProto integration via service layer
- 81-tool MCP (Model Context Protocol) system for AI interactions
- Multi-step onboarding flow
- URL-based settings modal system
- Deep link authentication handoff
- Cross-platform desktop compatibility (mobile not supported in product docs yet)

## Entry Points

| File            | Purpose                                                              |
| --------------- | -------------------------------------------------------------------- |
| `main.tsx`      | React root, polyfill imports, lazy deep link listener init           |
| `App.tsx`       | Provider chain: Redux → PersistGate → Socket → Telegram → HashRouter |
| `AppRoutes.tsx` | Route definitions with route guards                                  |
| `polyfills.ts`  | Node.js polyfills (Buffer, process, util) for telegram npm package   |

## Provider Chain

The application wraps components in a specific order due to dependencies:

```
Redux Provider
  └─ PersistGate (rehydrate auth + telegram state from localStorage)
      └─ UserProvider
          └─ SocketProvider (manages Socket.io connection + MCP init)
              └─ TelegramProvider (manages MTProto connection)
                  └─ HashRouter
                      └─ AppRoutes (route definitions + SettingsModal overlay)
```

**Why this order matters:**

1. Redux must be outermost for state access
2. PersistGate rehydrates persisted state before rendering
3. SocketProvider depends on Redux auth token
4. TelegramProvider depends on Redux telegram state
5. HashRouter provides navigation context to all routes

## Module Relationships

```
main.tsx (entry)
  ↓
App.tsx (providers chain)
  ├─ Redux Store ←→ Persist
  ├─ SocketProvider
  │   ├─ listens to auth.token changes
  │   ├─ calls socketService.connect(token)
  │   └─ init MCP server when socket connected
  ├─ TelegramProvider
  │   ├─ listens to telegram state
  │   ├─ calls mtprotoService.initialize(userId)
  │   └─ exposes useTelegram hook
  └─ HashRouter + AppRoutes
      ├─ ProtectedRoute ← checks Redux auth + onboarded
      ├─ PublicRoute ← redirects authenticated users
      ├─ pages/Home
      │   ├─ uses useUser hook → calls userApi
      │   └─ uses useNavigate → opens /settings
      ├─ pages/Login
      │   ├─ uses TelegramLoginButton
      │   └─ calls deeplink.ts functions
      └─ SettingsModal
          ├─ listens to location.pathname
          ├─ uses useSettingsNavigation hook
          └─ renders panels with Redux state
```

## Services Layer

```
Services Layer:
  ├─ apiClient (singleton)
  │   ├─ reads auth.token from Redux
  │   └─ makes HTTP requests to BACKEND_URL
  ├─ socketService (singleton)
  │   ├─ manages Socket.io connection
  │   └─ emits/listens for MCP messages
  └─ mtprotoService (singleton)
      ├─ manages TelegramClient
      └─ stores session in Redux telegram.byUser[userId].sessionString
```

## MCP System

```
MCP System:
  ├─ TelegramMCPServer (instantiated in SocketProvider)
  ├─ SocketIOMCPTransport (wraps socketService)
  └─ 81 tools in telegram/tools/
      ├─ each tool calls mtprotoService
      └─ results returned via socket transport to backend MCP client
```

## Data Flow

### Authentication Flow (Deep Link)

1. User authenticates in browser → receives `loginToken`
2. Browser redirects to `openhuman://auth?token=<loginToken>`
3. Desktop app catches deep link via Tauri plugin
4. `desktopDeepLinkListener` invokes Rust `exchange_token` command
5. Rust calls backend `POST /auth/desktop-exchange` (CORS-free)
6. Backend returns `{ sessionToken, user }`
7. App stores session in Redux, navigates to `/onboarding` or `/home`

### Socket.io Connection Flow

1. SocketProvider detects `auth.token` change
2. Calls `socketService.connect(token)`
3. On successful connection, initializes MCP server
4. MCP server registers 81 Telegram tools
5. Backend can invoke tools via JSON-RPC over Socket.io

### Telegram Connection Flow

1. TelegramProvider detects auth state
2. Calls `mtprotoService.initialize(userId)` and `connect()` in parallel
3. MTProto client authenticates with Telegram servers
4. Session string stored in Redux `telegram.byUser[userId].sessionString`
5. Chats/messages fetched and stored in Redux

## Key Patterns

### No localStorage Usage

- **Rule**: Avoid `localStorage`; use Redux with persistence instead
- **Exceptions**: Redux-persist's storage adapter (persistence layer)
- **Telegram session**: Stored in `telegram.byUser[userId].sessionString`

### Route Guard Pattern

- **PublicRoute**: Redirects authenticated users away
- **ProtectedRoute**: Requires `token` and optionally `isOnboarded`
- **DefaultRedirect**: Fallback based on auth state

### Settings Modal Pattern

- Renders via `createPortal` when `location.pathname.startsWith('/settings')`
- URL-based navigation without affecting main route
- Uses `useSettingsNavigation` hook

### MCP Tool Pattern

Each tool exports a handler:

```typescript
export const toolName: TelegramMCPToolHandler = {
  call: async (args, { telegramClient, userId }) => {
    // Perform Telegram API operation
    return { success: true, data: result };
  },
};
```

## File Organization

### Feature-Based Structure

- `pages/` - Full-page route components
- `pages/onboarding/` - Onboarding flow with steps
- `components/settings/` - Settings modal system
- `lib/mcp/telegram/tools/` - Individual MCP tools

### Slice-Based State

- `store/authSlice.ts` - Authentication
- `store/socketSlice.ts` - Socket connection
- `store/userSlice.ts` - User profile
- `store/telegram/` - Complex Telegram state (5 files)

### Singleton Services

- `services/socketService.ts` - Socket.io
- `services/mtprotoService.ts` - Telegram MTProto
- `services/apiClient.ts` - HTTP REST

---

_Next: [State Management](./02-state-management.md)_
