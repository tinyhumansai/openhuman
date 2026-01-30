# State Management

The application uses Redux Toolkit with Redux-Persist for robust state management.

## Store Configuration

**File:** `store/index.ts`

```typescript
// Combines all slices with persistence
const persistConfig = {
  key: 'root',
  storage,
  whitelist: ['auth', 'telegram']  // Persisted slices
};
```

## Redux State Structure

```typescript
RootState = {
  auth: {
    token: string | null,                      // JWT (persisted)
    isOnboardedByUser: Record<string, boolean> // Per-user flag (persisted)
  },
  socket: {
    byUser: Record<string, {                   // Per user ID
      status: "connecting" | "connected" | "disconnected",
      socketId: string | null
    }>
  },
  user: {
    profile: User | null,
    loading: boolean,
    error: string | null
  },
  telegram: {
    byUser: Record<string, TelegramState>     // Per Telegram user (persisted)
  }
}
```

## Slices

### Auth Slice (`store/authSlice.ts`)

Manages JWT token and per-user onboarding status.

**State:**
```typescript
interface AuthState {
  token: string | null;
  isOnboardedByUser: Record<string, boolean>;
}
```

**Actions:**
- `setToken(token: string)` - Store JWT after login
- `clearToken()` - Remove token on logout
- `setOnboarded({ userId, isOnboarded })` - Mark user as onboarded

**Selectors (`store/authSelectors.ts`):**
- `selectToken` - Get current JWT
- `selectIsOnboarded(userId)` - Check if user completed onboarding

### Socket Slice (`store/socketSlice.ts`)

Tracks Socket.io connection status per user.

**State:**
```typescript
interface SocketState {
  byUser: Record<string, {
    status: 'connecting' | 'connected' | 'disconnected';
    socketId: string | null;
  }>;
}
```

**Actions:**
- `setSocketStatus({ userId, status })` - Update connection status
- `setSocketId({ userId, socketId })` - Store socket ID
- `clearSocketState(userId)` - Clear user's socket state

**Selectors (`store/socketSelectors.ts`):**
- `selectSocketStatus(userId)` - Get connection status
- `selectIsSocketConnected(userId)` - Boolean connected check

### User Slice (`store/userSlice.ts`)

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
- `setUser(user)` - Store user profile
- `setUserLoading(loading)` - Set loading state
- `setUserError(error)` - Set error state
- `clearUser()` - Clear profile on logout

### Telegram Slice (`store/telegram/`)

Complex nested state management for Telegram integration.

**Files:**
- `index.ts` - Slice exports (actions, thunks)
- `types.ts` - Entity and state interfaces
- `reducers.ts` - Synchronous reducers
- `extraReducers.ts` - Async thunk handlers
- `thunks.ts` - Async operations

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
- `setCurrentUser` - Store authenticated Telegram user
- `setSessionString` - Store MTProto session (for persistence)
- `setConnectionStatus` - Update connection state
- `setAuthStatus` - Update authentication state
- `addChat` / `updateChat` - Manage chat list
- `addMessage` / `updateMessage` - Manage message history
- `setThreads` - Store thread data

**Thunks (`store/telegram/thunks.ts`):**
- `initializeTelegram(userId)` - Initialize MTProto client
- `connectTelegram(userId)` - Establish Telegram connection
- `fetchChats(userId)` - Load chat list
- `fetchMessages({ userId, chatId })` - Load message history
- `disconnectTelegram(userId)` - Clean disconnect

**Selectors (`store/telegramSelectors.ts`):**
- `selectTelegramState(userId)` - Get full Telegram state
- `selectTelegramConnectionStatus(userId)` - Get connection status
- `selectTelegramAuthStatus(userId)` - Get auth status
- `selectTelegramChats(userId)` - Get chat list
- `selectTelegramMessages(userId, chatId)` - Get messages for chat

## Typed Hooks

**File:** `store/hooks.ts`

```typescript
// Use these instead of plain useDispatch/useSelector
export const useAppDispatch: () => AppDispatch = useDispatch;
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector;
```

## Persistence Configuration

### What's Persisted
- `auth.token` - JWT for authentication
- `auth.isOnboardedByUser` - Per-user onboarding status
- `telegram.byUser` - Telegram state (sessions, chats, etc.)

### What's NOT Persisted
- `socket` - Connection state (reconnects on app start)
- `user.loading` / `user.error` - Transient UI states
- Telegram loading/error states

### Storage Backend
Redux-Persist uses localStorage adapter by default. This is the ONLY acceptable use of localStorage in the application.

## Usage Examples

### Reading State
```typescript
import { useAppSelector } from '../store/hooks';

function MyComponent() {
  const token = useAppSelector((state) => state.auth.token);
  const isConnected = useAppSelector((state) =>
    state.socket.byUser[userId]?.status === 'connected'
  );
  const chats = useAppSelector((state) =>
    state.telegram.byUser[userId]?.chats
  );
}
```

### Dispatching Actions
```typescript
import { useAppDispatch } from '../store/hooks';
import { setToken, clearToken } from '../store/authSlice';
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

### Using Selectors
```typescript
import { useAppSelector } from '../store/hooks';
import { selectIsOnboarded } from '../store/authSelectors';
import { selectTelegramConnectionStatus } from '../store/telegramSelectors';

function MyComponent({ userId }) {
  const isOnboarded = useAppSelector((state) => selectIsOnboarded(state, userId));
  const connectionStatus = useAppSelector((state) =>
    selectTelegramConnectionStatus(state, userId)
  );
}
```

## Best Practices

1. **Always use typed hooks** - `useAppDispatch` and `useAppSelector`
2. **Use selectors for derived state** - Memoized and testable
3. **Keep thunks in separate files** - Better organization
4. **Per-user state scoping** - Key state by user ID
5. **Avoid localStorage** - Use Redux-Persist instead

---

*Previous: [Architecture Overview](./01-architecture.md) | Next: [Services Layer](./03-services.md)*
