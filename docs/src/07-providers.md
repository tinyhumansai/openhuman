# Providers

React context providers manage service lifecycle and provide shared state.

## Provider Chain

The providers wrap the application in a specific order:

```typescript
// App.tsx
<Provider store={store}>
  <PersistGate loading={null} persistor={persistor}>
    <UserProvider>
      <SocketProvider>
        <TelegramProvider>
          <HashRouter>
            <AppRoutes />
          </HashRouter>
        </TelegramProvider>
      </SocketProvider>
    </UserProvider>
  </PersistGate>
</Provider>
```

**Order matters because:**

1. Redux must be outermost for state access
2. PersistGate rehydrates state before rendering children
3. SocketProvider depends on Redux auth token
4. TelegramProvider depends on Redux telegram state
5. HashRouter provides navigation to all routes

## SocketProvider (`providers/SocketProvider.tsx`)

Manages Socket.io connection lifecycle and MCP initialization.

### Responsibilities

- Auto-connect when auth token is available
- Auto-disconnect when token is cleared
- Initialize MCP server when socket connects
- Update Redux with connection status

### Implementation

```typescript
interface SocketContextValue {
  socket: Socket | null;
  isConnected: boolean;
  emit: (event: string, data: unknown) => void;
  on: (event: string, handler: Function) => void;
  off: (event: string, handler: Function) => void;
}

export function SocketProvider({ children }) {
  const token = useAppSelector((state) => state.auth.token);
  const userId = useAppSelector((state) => state.user.profile?.id);
  const dispatch = useAppDispatch();

  useEffect(() => {
    if (!token || !userId) {
      socketService.disconnect();
      dispatch(setSocketStatus({ userId, status: 'disconnected' }));
      return;
    }

    // Connect with auth token
    socketService.connect(token);
    dispatch(setSocketStatus({ userId, status: 'connecting' }));

    // Handle connection events
    socketService.on('connect', () => {
      dispatch(setSocketStatus({ userId, status: 'connected' }));
      dispatch(setSocketId({ userId, socketId: socketService.getSocket()?.id }));

      // Initialize MCP server
      initMCPServer(socketService.getSocket());
    });

    socketService.on('disconnect', () => {
      dispatch(setSocketStatus({ userId, status: 'disconnected' }));
      cleanupMCP();
    });

    socketService.on('connect_error', (error) => {
      console.error('Socket connection error:', error);
      dispatch(setSocketStatus({ userId, status: 'disconnected' }));
    });

    return () => {
      socketService.disconnect();
      cleanupMCP();
    };
  }, [token, userId]);

  const contextValue: SocketContextValue = {
    socket: socketService.getSocket(),
    isConnected: socketService.isConnected(),
    emit: socketService.emit.bind(socketService),
    on: socketService.on.bind(socketService),
    off: socketService.off.bind(socketService)
  };

  return (
    <SocketContext.Provider value={contextValue}>
      {children}
    </SocketContext.Provider>
  );
}
```

### Usage

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

## TelegramProvider (`providers/TelegramProvider.tsx`)

Manages Telegram MTProto connection lifecycle.

### Responsibilities

- Initialize MTProto client when user is authenticated
- Connect to Telegram servers
- Store session string in Redux
- Provide Telegram context to children

### Implementation

```typescript
interface TelegramContextValue {
  client: TelegramClient | null;
  connectionStatus: ConnectionStatus;
  authStatus: AuthStatus;
  connect: () => Promise<void>;
  disconnect: () => Promise<void>;
}

export function TelegramProvider({ children }) {
  const dispatch = useAppDispatch();
  const userId = useAppSelector((state) => state.user.profile?.id);
  const telegramState = useAppSelector((state) =>
    state.telegram.byUser[userId]
  );

  useEffect(() => {
    if (!userId) return;

    // Parallel initialization for faster startup
    const init = async () => {
      try {
        // Initialize and connect in parallel
        await Promise.all([
          dispatch(initializeTelegram(userId)).unwrap(),
          dispatch(connectTelegram(userId)).unwrap()
        ]);
      } catch (error) {
        console.error('Telegram initialization failed:', error);
      }
    };

    init();

    return () => {
      dispatch(disconnectTelegram(userId));
    };
  }, [userId]);

  // Restore session from persisted state
  useEffect(() => {
    if (telegramState?.sessionString) {
      const client = mtprotoService.getInstance().getClient();
      if (client) {
        client.setSession(telegramState.sessionString);
      }
    }
  }, [telegramState?.sessionString]);

  const contextValue: TelegramContextValue = {
    client: mtprotoService.getInstance().getClient(),
    connectionStatus: telegramState?.connectionStatus || 'disconnected',
    authStatus: telegramState?.authStatus || 'not_authenticated',
    connect: () => dispatch(connectTelegram(userId)).unwrap(),
    disconnect: () => dispatch(disconnectTelegram(userId)).unwrap()
  };

  return (
    <TelegramContext.Provider value={contextValue}>
      {children}
    </TelegramContext.Provider>
  );
}
```

### Usage

```typescript
import { useTelegram } from '../providers/TelegramProvider';

function ChatList() {
  const { client, connectionStatus, authStatus } = useTelegram();
  const [chats, setChats] = useState([]);

  useEffect(() => {
    if (connectionStatus === 'connected' && authStatus === 'authenticated') {
      const fetchChats = async () => {
        const dialogs = await client.getDialogs({ limit: 20 });
        setChats(dialogs);
      };
      fetchChats();
    }
  }, [client, connectionStatus, authStatus]);

  if (connectionStatus !== 'connected') {
    return <div>Connecting to Telegram...</div>;
  }

  return (
    <ul>
      {chats.map((chat) => (
        <li key={chat.id}>{chat.title}</li>
      ))}
    </ul>
  );
}
```

## UserProvider (`providers/UserProvider.tsx`)

Minimal user context provider (most user state is in Redux).

### Responsibilities

- Legacy user context for compatibility
- May be deprecated in favor of Redux

### Implementation

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

### Usage

```typescript
import { useUserContext } from '../providers/UserProvider';

function Header() {
  const { user, loading } = useUserContext();

  if (loading) return <Skeleton />;
  if (!user) return null;

  return <span>Welcome, {user.firstName}</span>;
}
```

## Provider Patterns

### Effect-Based Lifecycle

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

### Redux Integration

Providers read from and dispatch to Redux:

```typescript
// Read state
const token = useAppSelector(state => state.auth.token);

// Dispatch actions
const dispatch = useAppDispatch();
dispatch(setStatus({ userId, status: 'connected' }));
```

### Parallel Initialization

TelegramProvider runs init and connect in parallel:

```typescript
await Promise.all([
  dispatch(initializeTelegram(userId)).unwrap(),
  dispatch(connectTelegram(userId)).unwrap(),
]);
```

This reduces startup time compared to sequential operations.

### Session Restoration

Providers restore persisted state on mount:

```typescript
useEffect(() => {
  if (persistedSession) {
    service.restoreSession(persistedSession);
  }
}, [persistedSession]);
```

## Context vs Redux

| Use Context For                    | Use Redux For                      |
| ---------------------------------- | ---------------------------------- |
| Service instances (socket, client) | Serializable state (status, data)  |
| Methods (emit, on, off)            | Persisted state (sessions, tokens) |
| Derived values                     | Complex state logic                |

Example:

- `SocketContext` provides `socket` instance and `emit` method
- Redux stores `socketStatus` and `socketId`

## Testing Providers

### Mock Provider for Tests

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

### Testing Provider Effects

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

---

_Previous: [Components](./06-components.md) | Next: [Hooks & Utils](./08-hooks-utils.md)_
