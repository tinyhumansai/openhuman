# Services Layer

The application uses singleton services for external communication. This prevents connection leaks and provides consistent API access.

## Service Architecture

```
Services Layer
  ├─ apiClient (HTTP REST)
  │   ├─ reads auth.token from Redux
  │   └─ makes requests to BACKEND_URL
  ├─ socketService (Socket.io)
  │   ├─ manages real-time connection
  │   └─ emits/listens for MCP messages
  └─ mtprotoService (Telegram)
      ├─ manages TelegramClient
      └─ stores session in Redux
```

## API Client (`services/apiClient.ts`)

HTTP REST client for backend communication.

### Features
- Fetch-based implementation
- Auto-injects JWT from Redux store
- Typed request/response handling
- Error handling with typed errors

### Usage
```typescript
import apiClient from '../services/apiClient';

// GET request
const user = await apiClient.get<User>('/users/me');

// POST request
const result = await apiClient.post<LoginResponse>('/auth/login', {
  email,
  password
});

// With custom headers
const data = await apiClient.get<Data>('/endpoint', {
  headers: { 'X-Custom': 'value' }
});
```

### Configuration
Reads `VITE_BACKEND_URL` from environment or uses default:
```typescript
const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || 'https://api.example.com';
```

## API Endpoints (`services/api/`)

### Auth API (`services/api/authApi.ts`)

Authentication-related endpoints.

```typescript
import { authApi } from '../services/api/authApi';

// Login
const { token, user } = await authApi.login(credentials);

// Token exchange (for deep link flow)
const { sessionToken, user } = await authApi.exchangeToken(loginToken);

// Logout
await authApi.logout();
```

### User API (`services/api/userApi.ts`)

User profile endpoints.

```typescript
import { userApi } from '../services/api/userApi';

// Get current user
const user = await userApi.getCurrentUser();

// Update profile
const updated = await userApi.updateProfile({ firstName, lastName });

// Get settings
const settings = await userApi.getSettings();
```

## Socket Service (`services/socketService.ts`)

Socket.io client singleton for real-time communication.

### Features
- Singleton pattern - single connection per app
- Auth token passed in socket `auth` object
- Transports: polling first, then WebSocket upgrade
- Auto-reconnection handling

### API
```typescript
import socketService from '../services/socketService';

// Connect with auth token
socketService.connect(token);

// Disconnect
socketService.disconnect();

// Emit event
socketService.emit('event-name', data);

// Listen for events
socketService.on('event-name', (data) => {
  // Handle event
});

// Remove listener
socketService.off('event-name', handler);

// One-time listener
socketService.once('event-name', (data) => {
  // Handle once
});

// Get socket instance
const socket = socketService.getSocket();

// Check connection status
const isConnected = socketService.isConnected();
```

### Connection Flow
```typescript
// In SocketProvider.tsx
useEffect(() => {
  if (token) {
    socketService.connect(token);

    socketService.on('connect', () => {
      dispatch(setSocketStatus({ userId, status: 'connected' }));
      dispatch(setSocketId({ userId, socketId: socket.id }));
      // Initialize MCP server
      initMCPServer(socketService.getSocket());
    });

    socketService.on('disconnect', () => {
      dispatch(setSocketStatus({ userId, status: 'disconnected' }));
    });
  }

  return () => {
    socketService.disconnect();
  };
}, [token]);
```

### Configuration
```typescript
const socket = io(BACKEND_URL, {
  auth: { token },
  transports: ['polling', 'websocket'],
  reconnection: true,
  reconnectionAttempts: 5,
  reconnectionDelay: 1000
});
```

## MTProto Service (`services/mtprotoService.ts`)

Telegram MTProto client singleton.

### Features
- Singleton pattern - one client per user
- Session persistence via Redux (not localStorage)
- Auto-retry for FLOOD_WAIT up to 60s
- Supports QR login and phone auth

### Initialization
```typescript
import mtprotoService from '../services/mtprotoService';

// Get or create instance for user
const client = await mtprotoService.getInstance().initialize(userId);

// Set session string (from Redux)
await client.setSession(sessionString);

// Connect to Telegram
await client.connect();
```

### Session Management
```typescript
// Session is stored in Redux, not localStorage
// In TelegramProvider:
const sessionString = useAppSelector((state) =>
  state.telegram.byUser[userId]?.sessionString
);

// When session updates
useEffect(() => {
  if (client && sessionString) {
    client.setSession(sessionString);
  }
}, [sessionString]);

// Save session after auth
const newSession = await client.getSession();
dispatch(setSessionString({ userId, sessionString: newSession }));
```

### API Operations
```typescript
// Get current user
const me = await client.getMe();

// Get dialogs (chats)
const dialogs = await client.getDialogs({ limit: 20 });

// Send message
await client.sendMessage(peer, { message: 'Hello!' });

// Get history
const messages = await client.getMessages(peer, { limit: 50 });
```

### Error Handling
```typescript
try {
  await client.connect();
} catch (error) {
  if (error.message.includes('FLOOD_WAIT')) {
    const seconds = parseInt(error.message.match(/\d+/)?.[0] || '60');
    if (seconds <= 60) {
      // Auto-retry after wait
      await new Promise(r => setTimeout(r, seconds * 1000));
      await client.connect();
    }
  }
  throw error;
}
```

## Service Integration with Providers

### SocketProvider
```typescript
// providers/SocketProvider.tsx
export function SocketProvider({ children }) {
  const token = useAppSelector((state) => state.auth.token);

  useEffect(() => {
    if (token) {
      socketService.connect(token);
      // On connect, initialize MCP
    }
    return () => socketService.disconnect();
  }, [token]);

  return <SocketContext.Provider value={...}>{children}</SocketContext.Provider>;
}
```

### TelegramProvider
```typescript
// providers/TelegramProvider.tsx
export function TelegramProvider({ children }) {
  const dispatch = useAppDispatch();
  const userId = useAppSelector((state) => state.user.profile?.id);

  useEffect(() => {
    if (userId) {
      // Parallel init + connect for faster startup
      Promise.all([
        dispatch(initializeTelegram(userId)),
        dispatch(connectTelegram(userId))
      ]);
    }
  }, [userId]);

  return <TelegramContext.Provider value={...}>{children}</TelegramContext.Provider>;
}
```

## Best Practices

1. **Use singletons** - Never create multiple service instances
2. **Store sessions in Redux** - Not localStorage
3. **Clean up on unmount** - Disconnect in useEffect cleanup
4. **Handle errors gracefully** - Retry for transient failures
5. **Pass auth via proper channels** - Socket auth object, not query string

---

*Previous: [State Management](./02-state-management.md) | Next: [MCP System](./04-mcp-system.md)*
