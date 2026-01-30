# Rust Backend Architecture

## Overview

The Tauri Rust backend provides native functionality for the AlphaHuman desktop application:
- System tray with background execution
- Deep link authentication
- Secure session storage (OS Keychain)
- Socket.io state management
- Native notifications
- Window management

## Directory Structure

```
src-tauri/src/
├── lib.rs                      # Entry point, plugin registration, tray setup
├── main.rs                     # Binary entry (desktop)
├── commands/                   # Tauri IPC commands
│   ├── mod.rs
│   ├── auth.rs                 # Authentication commands
│   ├── socket.rs               # Socket state commands
│   ├── telegram.rs             # Telegram login commands
│   └── window.rs               # Window management commands
├── services/                   # Background services
│   ├── mod.rs
│   ├── session_service.rs      # Secure session storage (keychain)
│   ├── socket_service.rs       # Socket.io state management
│   └── notification_service.rs # Native notifications
├── models/                     # Data structures
│   ├── mod.rs
│   ├── auth.rs                 # Auth types (Session, User)
│   └── socket.rs               # Socket types (ConnectionStatus)
└── utils/                      # Configuration and helpers
    ├── mod.rs
    └── config.rs               # Environment configuration
```

## Key Components

### lib.rs - Application Entry

The main entry point that:
1. Registers Tauri plugins (opener, deep-link, autostart, notification)
2. Sets up system tray with Show/Hide and Quit menu
3. Handles macOS-specific window close behavior (minimize to tray)
4. Registers all IPC commands

### Commands Layer

Commands are exposed to the frontend via Tauri's IPC:

| Module | Commands | Purpose |
|--------|----------|---------|
| `auth` | `exchange_token`, `get_auth_state`, `logout`, etc. | Authentication |
| `socket` | `socket_connect`, `report_socket_connected`, etc. | Socket state |
| `telegram` | `start_telegram_login` | Telegram OAuth |
| `window` | `show_window`, `hide_window`, `toggle_window`, etc. | Window control |

### Services Layer

Singleton services providing background functionality:

| Service | Purpose | Storage |
|---------|---------|---------|
| `SessionService` | Secure auth token storage | OS Keychain |
| `SocketService` | Socket.io state management | Memory + Events |
| `NotificationService` | Native notifications | N/A |

## Architecture Decisions

### Socket.io Strategy

The frontend maintains the actual Socket.io connection, while Rust:
1. Stores connection parameters
2. Tracks connection state
3. Emits events to coordinate with frontend
4. Ensures socket stays connected when window is hidden

This approach is necessary because:
- Rust's Socket.io libraries have API compatibility issues
- The WebView maintains state when hidden (unlike browser tabs)
- Frontend JavaScript is better suited for Socket.io's event-driven model

### Keychain Storage

Session tokens are stored in the OS keychain for security:
- **macOS**: Keychain
- **Windows**: Credential Manager
- **Linux**: Secret Service

### Event Bridge Pattern

Rust communicates with the frontend via Tauri events:

```rust
// Rust emits event
app.emit("socket:should_connect", json!({ "backendUrl": url, "token": token }))

// Frontend listens
listen("socket:should_connect", (event) => {
  socketService.connect(event.payload.backendUrl, event.payload.token);
});
```

## Plugin Dependencies

| Plugin | Version | Purpose |
|--------|---------|---------|
| `tauri-plugin-opener` | 2 | Open URLs in browser |
| `tauri-plugin-deep-link` | 2.0.0 | Handle `alphahuman://` URLs |
| `tauri-plugin-autostart` | 2 | Launch at login |
| `tauri-plugin-notification` | 2 | Native notifications |

## Cargo Dependencies

| Crate | Purpose |
|-------|---------|
| `tauri` | Core framework with tray and macOS APIs |
| `serde`, `serde_json` | Serialization |
| `reqwest` | HTTP client |
| `tokio` | Async runtime |
| `keyring` | Secure credential storage |
| `once_cell` | Lazy static singletons |
| `parking_lot` | Fast mutexes |
| `log`, `env_logger` | Logging |

## Platform-Specific Behavior

### macOS
- Window close button hides instead of quitting
- Dock icon click shows window
- LaunchAgent for autostart
- Keychain for secure storage

### Windows/Linux
- Deep link registration at runtime
- Platform-specific credential storage
- Registry (Windows) or desktop file (Linux) autostart

---

*Next: [Commands Reference](./02-commands.md)*
