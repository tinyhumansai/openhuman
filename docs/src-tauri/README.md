# Rust Backend Documentation

## Overview

This documentation covers the Tauri Rust backend for the AlphaHuman desktop application.

## Quick Reference

| Document | Description |
|----------|-------------|
| [Architecture](./01-architecture.md) | System architecture and module structure |
| [Commands Reference](./02-commands.md) | All Tauri IPC commands |
| [Services](./03-services.md) | Background services documentation |

## Features Implemented

1. **System Tray** - Background execution with menu bar icon
2. **Telegram Widget Login** → Deep link session creation
3. **Socket.io State Management** - Persistent background connection
4. **Secure Session Storage** - OS Keychain integration
5. **Native Notifications** - Desktop notifications
6. **Cross-Platform** - macOS, Windows, Linux ready

## Current State Analysis

### Existing Implementation (`lib.rs`)
- ✅ System tray with show/hide/quit
- ✅ Deep link handling (`alphahuman://` scheme)
- ✅ Token exchange command (CORS bypass)
- ✅ Autostart plugin (macOS LaunchAgent)
- ✅ Window minimize-to-tray on close (macOS)

### Missing Features
- ❌ Socket.io client in Rust (background persistence)
- ❌ Telegram Widget integration
- ❌ Session management in Rust
- ❌ Background service architecture
- ❌ Notification system
- ❌ State persistence (keychain/secure storage)

---

## Implementation Plan

### Phase 1: Project Structure Refactoring

**Goal**: Modular architecture for maintainability

```
src-tauri/src/
├── lib.rs                    # Entry point, plugin registration
├── main.rs                   # Binary entry (unchanged)
├── commands/                 # Tauri commands (IPC)
│   ├── mod.rs
│   ├── auth.rs               # Token exchange, session management
│   ├── socket.rs             # Socket connection control
│   └── telegram.rs           # Telegram-specific commands
├── services/                 # Background services
│   ├── mod.rs
│   ├── socket_service.rs     # Persistent Socket.io client
│   ├── session_service.rs    # Secure session storage
│   └── notification_service.rs # Native notifications
├── models/                   # Data structures
│   ├── mod.rs
│   ├── auth.rs               # Auth types
│   └── socket.rs             # Socket message types
└── utils/                    # Helpers
    ├── mod.rs
    └── config.rs             # Environment configuration
```

### Phase 2: Socket.io Background Service

**Goal**: Persistent WebSocket connection even when app is in background

**Dependencies to add:**
```toml
[dependencies]
rust_socketio = "0.6"           # Socket.io client
tokio = { version = "1", features = ["full", "sync"] }
once_cell = "1.19"              # Lazy static for singleton
parking_lot = "0.12"            # Fast mutexes
```

**Implementation:**
```rust
// services/socket_service.rs
pub struct SocketService {
    client: Option<Client>,
    auth_token: Option<String>,
    is_connected: AtomicBool,
}

impl SocketService {
    pub async fn connect(&self, token: &str) -> Result<(), Error>;
    pub async fn disconnect(&self) -> Result<(), Error>;
    pub async fn emit(&self, event: &str, data: Value) -> Result<(), Error>;
    pub fn is_connected(&self) -> bool;
}
```

**Background Persistence:**
- Socket runs on Tokio runtime, independent of window state
- Connection survives window hide/minimize
- Auto-reconnect on network recovery
- Heartbeat/ping to keep connection alive

### Phase 3: Telegram Widget Login Flow

**Goal**: Web-based Telegram auth → Deep link callback → Native session

**Flow:**
```
1. User clicks "Login with Telegram" in desktop app
   ↓
2. App opens system browser to:
   ${BACKEND_URL}/auth/telegram-widget?redirect=alphahuman://auth
   ↓
3. Backend serves Telegram Login Widget HTML page
   ↓
4. User authenticates with Telegram
   ↓
5. Telegram callback → Backend validates → Creates loginToken
   ↓
6. Backend redirects to: alphahuman://auth?token={loginToken}
   ↓
7. Desktop app catches deep link
   ↓
8. Rust `exchange_token` → Backend exchanges for sessionToken
   ↓
9. Session stored securely (Keychain on macOS)
   ↓
10. Socket connects with session token
```

**Backend Endpoint Needed:**
```
GET /auth/telegram-widget?redirect={deeplink_scheme}
```
Returns HTML page with Telegram Login Widget that redirects to the specified scheme.

**Commands to implement:**
```rust
#[tauri::command]
async fn start_telegram_login(app: AppHandle) -> Result<(), String> {
    // Open browser to Telegram widget page
    let url = format!("{}/auth/telegram-widget?redirect=alphahuman://auth", BACKEND_URL);
    opener::open(&url)?;
    Ok(())
}

#[tauri::command]
async fn get_session() -> Result<Option<SessionInfo>, String> {
    // Return current session from secure storage
}

#[tauri::command]
async fn logout(app: AppHandle) -> Result<(), String> {
    // Clear session, disconnect socket
}
```

### Phase 4: Secure Session Storage

**Goal**: Store auth tokens securely using OS keychain

**Dependencies:**
```toml
[dependencies]
keyring = "3"  # Cross-platform keychain access
```

**Implementation:**
```rust
// services/session_service.rs
pub struct SessionService {
    keyring: Entry,
}

impl SessionService {
    const SERVICE: &'static str = "com.megamind.tauri-app";

    pub fn store_token(&self, token: &str) -> Result<(), Error>;
    pub fn get_token(&self) -> Result<Option<String>, Error>;
    pub fn clear_token(&self) -> Result<(), Error>;
}
```

**Platform Support:**
- macOS: Keychain
- Windows: Credential Manager
- Linux: Secret Service (libsecret)

### Phase 5: Native Notifications

**Goal**: Show notifications even when app is minimized

**Dependencies:**
```toml
[dependencies]
tauri-plugin-notification = "2"
```

**Capability Addition:**
```json
{
  "permissions": [
    "notification:default",
    "notification:allow-notify",
    "notification:allow-request-permission"
  ]
}
```

**Usage:**
```rust
// services/notification_service.rs
pub fn show_notification(title: &str, body: &str) -> Result<(), Error> {
    Notification::new()
        .title(title)
        .body(body)
        .show()?;
    Ok(())
}
```

### Phase 6: Event Bridge (Rust ↔ Frontend)

**Goal**: Bidirectional communication between Rust services and React frontend

**Events from Rust to Frontend:**
```rust
// Emit to frontend when socket receives message
app.emit("socket:message", payload)?;
app.emit("socket:connected", ())?;
app.emit("socket:disconnected", ())?;
app.emit("telegram:notification", notification)?;
```

**Frontend listening:**
```typescript
import { listen } from '@tauri-apps/api/event';

await listen('socket:message', (event) => {
  // Handle message from Rust socket service
});
```

### Phase 7: MCP Integration in Rust

**Goal**: Run MCP tools from Rust for performance-critical operations

**Approach:**
- Keep MCP tools in TypeScript for flexibility
- Rust handles socket transport
- Frontend dispatches tool calls
- Rust forwards via socket, returns results

**Alternative (Full Rust MCP):**
- Implement tool handlers in Rust
- Higher performance, but more maintenance
- Consider for v2

---

## Implementation Order

| Phase | Priority | Effort | Dependencies |
|-------|----------|--------|--------------|
| 1. Project Structure | High | 2h | None |
| 2. Socket.io Service | High | 4h | Phase 1 |
| 3. Telegram Widget Login | High | 3h | Backend endpoint |
| 4. Secure Storage | High | 2h | Phase 1 |
| 5. Notifications | Medium | 1h | Phase 2 |
| 6. Event Bridge | High | 2h | Phase 2 |
| 7. MCP Integration | Low | 4h+ | Phase 2, 6 |

**Total Estimated Effort**: 18+ hours

---

## Cross-Platform Considerations

### macOS
- ✅ System tray (menu bar)
- ✅ LaunchAgent autostart
- ✅ Keychain storage
- ✅ Deep link via Info.plist
- ⚠️ Notarization for distribution

### Windows
- ✅ System tray
- ✅ Registry autostart
- ✅ Credential Manager storage
- ✅ Deep link via registry
- ⚠️ Code signing for SmartScreen

### Linux
- ✅ System tray (AppIndicator)
- ✅ Desktop file autostart
- ✅ Secret Service storage
- ⚠️ Deep link varies by desktop environment

### Mobile (Future)
- ❌ No system tray
- ❌ Different auth flow
- ❌ Push notifications instead of socket
- Consider separate implementation

---

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_socket_connect() { ... }

    #[test]
    fn test_session_storage() { ... }
}
```

### Integration Tests
- Deep link flow end-to-end
- Socket reconnection scenarios
- Background persistence verification

### Manual Testing
- Build debug `.app` bundle
- Test tray behavior
- Test window minimize/restore
- Test background socket

---

## Files to Create/Modify

### New Files
- `src-tauri/src/commands/mod.rs`
- `src-tauri/src/commands/auth.rs`
- `src-tauri/src/commands/socket.rs`
- `src-tauri/src/commands/telegram.rs`
- `src-tauri/src/services/mod.rs`
- `src-tauri/src/services/socket_service.rs`
- `src-tauri/src/services/session_service.rs`
- `src-tauri/src/services/notification_service.rs`
- `src-tauri/src/models/mod.rs`
- `src-tauri/src/models/auth.rs`
- `src-tauri/src/models/socket.rs`
- `src-tauri/src/utils/mod.rs`
- `src-tauri/src/utils/config.rs`

### Modified Files
- `src-tauri/Cargo.toml` - Add dependencies
- `src-tauri/src/lib.rs` - Refactor, use modules
- `src-tauri/capabilities/default.json` - Add permissions

---

## Success Criteria

1. ✅ User can log in via Telegram widget
2. ✅ Session persists across app restarts
3. ✅ Socket stays connected when app is minimized
4. ✅ Notifications appear for new messages
5. ✅ All web features work in desktop app
6. ✅ Cross-platform compatible architecture

---

*Plan created by stevenbaba - 2026-01-29*
