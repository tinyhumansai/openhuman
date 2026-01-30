# Services Documentation

This document describes the background services in the Rust backend.

## SessionService

Manages user sessions with secure OS keychain storage.

### Location
`src-tauri/src/services/session_service.rs`

### Purpose
- Store authentication tokens securely in OS keychain
- Cache session in memory for fast access
- Persist sessions across app restarts

### API

```rust
pub struct SessionService {
    // ...
}

impl SessionService {
    /// Create a new SessionService (loads from keychain)
    pub fn new() -> Self;

    /// Store a new session
    pub fn store_session(&self, token: &str, user: &User) -> Result<(), String>;

    /// Get the current session token
    pub fn get_token(&self) -> Option<String>;

    /// Get the current session
    pub fn get_session(&self) -> Option<Session>;

    /// Get the current user
    pub fn get_user(&self) -> Option<User>;

    /// Check if there's an active session
    pub fn is_authenticated(&self) -> bool;

    /// Clear the current session (logout)
    pub fn clear_session(&self) -> Result<(), String>;
}
```

### Keychain Storage

| Platform | Storage Backend |
|----------|-----------------|
| macOS | Keychain |
| Windows | Credential Manager |
| Linux | Secret Service (libsecret) |

### Stored Data

```json
{
  "token": "jwt-session-token",
  "user_id": "user-uuid",
  "user": {
    "id": "user-uuid",
    "firstName": "John",
    "lastName": "Doe"
  },
  "created_at": 1706540000,
  "expires_at": null
}
```

### Usage

```rust
use crate::commands::auth::SESSION_SERVICE;

// Store session
SESSION_SERVICE.store_session("token", &user)?;

// Get token
if let Some(token) = SESSION_SERVICE.get_token() {
    // Use token
}

// Check auth
if SESSION_SERVICE.is_authenticated() {
    // User is logged in
}

// Logout
SESSION_SERVICE.clear_session()?;
```

---

## SocketService

Manages Socket.io connection state and coordinates with the frontend.

### Location
`src-tauri/src/services/socket_service.rs`

### Purpose
- Track socket connection state
- Store connection parameters for reconnection
- Emit events to frontend for connection control
- Enable background socket persistence

### Architecture

The actual Socket.io client runs in the frontend (JavaScript). The Rust service:
1. Stores connection parameters (URL, token)
2. Tracks state reported by frontend
3. Emits events to request connect/disconnect
4. Enables socket to persist when window is hidden

### API

```rust
pub struct SocketService {
    // ...
}

impl SocketService {
    /// Create a new SocketService
    pub fn new() -> Self;

    /// Set app handle for event emission
    pub fn set_app_handle(&self, handle: AppHandle);

    /// Get current connection status
    pub fn get_status(&self) -> ConnectionStatus;

    /// Get current socket state
    pub fn get_state(&self) -> SocketState;

    /// Check if connected
    pub fn is_connected(&self) -> bool;

    /// Request frontend to connect
    pub fn request_connect(&self, backend_url: &str, token: &str) -> Result<(), String>;

    /// Request frontend to disconnect
    pub fn request_disconnect(&self) -> Result<(), String>;

    /// Update status (called by frontend via command)
    pub fn update_status(&self, status: ConnectionStatus, socket_id: Option<String>);

    /// Report connection (called by frontend)
    pub fn report_connected(&self, socket_id: Option<String>);

    /// Report disconnection (called by frontend)
    pub fn report_disconnected(&self);

    /// Report error (called by frontend)
    pub fn report_error(&self, error: &str);

    /// Get stored connection params for reconnection
    pub fn get_connection_params(&self) -> Option<(String, String)>;

    /// Clear stored credentials
    pub fn clear_credentials(&self);
}
```

### Events Emitted

| Event | Payload | When |
|-------|---------|------|
| `socket:should_connect` | `{ backendUrl, token }` | `request_connect` called |
| `socket:should_disconnect` | `()` | `request_disconnect` called |
| `socket:state_changed` | `SocketState` | State changes |
| `socket:error` | `string` | Error reported |

### Connection States

```rust
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error,
}
```

### Usage

```rust
use crate::services::socket_service::SOCKET_SERVICE;

// Initialize with app handle
SOCKET_SERVICE.set_app_handle(app.handle());

// Request connection (emits event to frontend)
SOCKET_SERVICE.request_connect("https://api.example.com", "token")?;

// Check status
if SOCKET_SERVICE.is_connected() {
    // Socket is connected
}

// Frontend reports status via commands
// invoke('report_socket_connected', { socketId: 'abc' })
```

### Background Persistence

When the window is hidden:
1. The Tauri app continues running (tray icon)
2. The WebView is not destroyed, just hidden
3. Socket.io connection in JavaScript stays active
4. Frontend continues receiving messages
5. User can show window to see updates

---

## NotificationService

Shows native desktop notifications.

### Location
`src-tauri/src/services/notification_service.rs`

### Purpose
- Show native notifications
- Check notification permission
- Request notification permission

### API

```rust
pub struct NotificationService;

impl NotificationService {
    /// Show a simple notification
    pub fn show(app: &AppHandle, title: &str, body: &str) -> Result<(), String>;

    /// Show a notification with an icon
    pub fn show_with_icon(
        app: &AppHandle,
        title: &str,
        body: &str,
        icon: &str,
    ) -> Result<(), String>;

    /// Show a notification for a new message
    pub fn show_message_notification(
        app: &AppHandle,
        sender: &str,
        message: &str,
    ) -> Result<(), String>;

    /// Check if notifications are permitted
    pub fn is_permission_granted(app: &AppHandle) -> Result<bool, String>;

    /// Request notification permission
    pub fn request_permission(app: &AppHandle) -> Result<bool, String>;
}
```

### Usage

```rust
use crate::services::notification_service::NotificationService;

// Show notification
NotificationService::show(&app, "New Message", "You have a new message")?;

// Show message notification
NotificationService::show_message_notification(&app, "John", "Hey, how are you?")?;

// Check permission
if NotificationService::is_permission_granted(&app)? {
    // Can show notifications
}
```

---

## Service Initialization

Services are initialized as singletons in their respective modules:

```rust
// In auth.rs
pub static SESSION_SERVICE: Lazy<Arc<SessionService>> =
    Lazy::new(|| Arc::new(SessionService::new()));

// In socket_service.rs
pub static SOCKET_SERVICE: Lazy<Arc<SocketService>> =
    Lazy::new(|| Arc::new(SocketService::new()));
```

The SocketService's app handle is set during app setup:

```rust
// In lib.rs setup()
SOCKET_SERVICE.set_app_handle(app.handle().clone());
```

---

*Previous: [Commands Reference](./02-commands.md) | [Back to Index](./README.md)*
