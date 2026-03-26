# Tauri Commands Reference

This document lists all Tauri commands available to the frontend.

## Authentication Commands

### `exchange_token`

Exchange a login token for a session token.

```typescript
const result = await invoke("exchange_token", {
  backendUrl: "https://api.example.com",
  token: "login-token-from-deep-link",
});
// Returns: { sessionToken: string, user: User }
```

### `get_auth_state`

Get current authentication state.

```typescript
const state = await invoke<AuthState>("get_auth_state");
// Returns: { is_authenticated: boolean, user: User | null }
```

### `get_session_token`

Get the current session token.

```typescript
const token = await invoke<string | null>("get_session_token");
```

### `get_current_user`

Get the current authenticated user.

```typescript
const user = await invoke<User | null>("get_current_user");
```

### `is_authenticated`

Check if user is authenticated.

```typescript
const isAuth = await invoke<boolean>("is_authenticated");
```

### `logout`

Clear session and disconnect socket.

```typescript
await invoke("logout");
```

### `store_session`

Manually store a session (usually called automatically by `exchange_token`).

```typescript
await invoke("store_session", {
  token: "session-token",
  user: { id: "123", firstName: "John" },
});
```

## Socket Commands

### `socket_connect`

Request frontend to connect to socket server.

```typescript
await invoke("socket_connect", {
  backendUrl: "https://api.example.com",
  token: "session-token",
});
// Emits "socket:should_connect" event
```

### `socket_disconnect`

Request frontend to disconnect from socket server.

```typescript
await invoke("socket_disconnect");
// Emits "socket:should_disconnect" event
```

### `get_socket_state`

Get current socket state.

```typescript
const state = await invoke<SocketState>("get_socket_state");
// Returns: { status: 'connected' | 'disconnected' | ..., socket_id: string | null, error: string | null }
```

### `is_socket_connected`

Check if socket is connected.

```typescript
const isConnected = await invoke<boolean>("is_socket_connected");
```

### `report_socket_connected`

Report that socket connected (called by frontend).

```typescript
await invoke("report_socket_connected", { socketId: "socket-id" });
```

### `report_socket_disconnected`

Report that socket disconnected (called by frontend).

```typescript
await invoke("report_socket_disconnected");
```

### `report_socket_error`

Report socket error (called by frontend).

```typescript
await invoke("report_socket_error", { error: "Connection failed" });
```

### `update_socket_status`

Update socket status (called by frontend).

```typescript
await invoke("update_socket_status", {
  status: "connected", // 'connected' | 'connecting' | 'disconnected' | 'reconnecting' | 'error'
  socketId: "socket-id",
});
```

## Telegram Commands

### `start_telegram_login`

Open Telegram login widget in browser.

```typescript
await invoke("start_telegram_login");
// Opens ${DEFAULT_BACKEND_URL}/auth/telegram-widget?redirect=openhuman://auth
```

### `start_telegram_login_with_url`

Open Telegram login widget with custom backend URL.

```typescript
await invoke("start_telegram_login_with_url", {
  backendUrl: "https://custom-backend.com",
});
```

## Window Commands

### `show_window`

Show and focus the main window.

```typescript
await invoke("show_window");
```

### `hide_window`

Hide the main window.

```typescript
await invoke("hide_window");
```

### `toggle_window`

Toggle window visibility.

```typescript
await invoke("toggle_window");
```

### `is_window_visible`

Check if window is visible.

```typescript
const isVisible = await invoke<boolean>("is_window_visible");
```

### `minimize_window`

Minimize the window.

```typescript
await invoke("minimize_window");
```

### `maximize_window`

Maximize or unmaximize the window.

```typescript
await invoke("maximize_window");
```

### `close_window`

Close the window (minimizes to tray on macOS).

```typescript
await invoke("close_window");
```

### `set_window_title`

Set the window title.

```typescript
await invoke("set_window_title", { title: "New Title" });
```

## Events (Rust → Frontend)

These events are emitted by Rust and can be listened to in the frontend:

### Socket Events

| Event                      | Payload                                 | Description             |
| -------------------------- | --------------------------------------- | ----------------------- |
| `socket:connected`         | `()`                                    | Socket connected        |
| `socket:disconnected`      | `()`                                    | Socket disconnected     |
| `socket:error`             | `string`                                | Socket error occurred   |
| `socket:message`           | `{ event: string, data: any }`          | Socket message received |
| `socket:state_changed`     | `SocketState`                           | Socket state changed    |
| `socket:should_connect`    | `{ backendUrl: string, token: string }` | Request to connect      |
| `socket:should_disconnect` | `()`                                    | Request to disconnect   |

### Listening to Events

```typescript
import { listen } from "@tauri-apps/api/event";

// Listen for socket connection request
await listen("socket:should_connect", (event) => {
  const { backendUrl, token } = event.payload;
  socketService.connect(backendUrl, token);
});

// Listen for state changes
await listen("socket:state_changed", (event) => {
  const state = event.payload as SocketState;
  dispatch(setSocketStatus(state.status));
});
```

## Type Definitions

```typescript
interface AuthState {
  is_authenticated: boolean;
  user: User | null;
}

interface User {
  id: string;
  firstName?: string;
  lastName?: string;
  username?: string;
  email?: string;
  telegramId?: string;
}

interface SocketState {
  status:
    | "connected"
    | "connecting"
    | "disconnected"
    | "reconnecting"
    | "error";
  socket_id: string | null;
  error?: string;
}
```

---

_Previous: [Architecture](./01-architecture.md) | Next: [Services](./03-services.md)_
