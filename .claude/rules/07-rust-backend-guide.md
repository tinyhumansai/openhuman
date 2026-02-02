# Rust Backend Guide

## Structure

```
src-tauri/
├── Cargo.toml          # Rust dependencies
├── build.rs            # Build script
├── tauri.conf.json     # Tauri configuration
├── capabilities/       # Permission configurations
├── icons/              # App icons
└── src/
    ├── lib.rs          # Library crate (for mobile)
    └── main.rs         # Binary crate (for desktop)
```

## Creating Commands

Commands allow the frontend to call Rust functions.

### Basic Command

```rust
// src-tauri/src/lib.rs

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Async Command

```rust
#[tauri::command]
async fn fetch_data(url: String) -> Result<String, String> {
    reqwest::get(&url)
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())
}
```

### Command with State

```rust
use std::sync::Mutex;
use tauri::State;

struct AppState {
    counter: Mutex<i32>,
}

#[tauri::command]
fn increment(state: State<AppState>) -> i32 {
    let mut counter = state.counter.lock().unwrap();
    *counter += 1;
    *counter
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            counter: Mutex::new(0),
        })
        .invoke_handler(tauri::generate_handler![increment])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

## Calling Commands from Frontend

```typescript
import { invoke } from '@tauri-apps/api/core';

// Basic call
const greeting = await invoke<string>('greet', { name: 'World' });

// With error handling
try {
  const data = await invoke<string>('fetch_data', { url: 'https://api.example.com' });
} catch (error) {
  console.error('Command failed:', error);
}
```

## Events

### Emit from Rust

```rust
use tauri::Emitter;

#[tauri::command]
fn start_process(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Do work...
        app.emit("process-complete", "Done!").unwrap();
    });
}
```

### Listen in Frontend

```typescript
import { listen } from '@tauri-apps/api/event';

const unlisten = await listen('process-complete', event => {
  console.log('Process completed:', event.payload);
});

// Later: unlisten();
```

## Adding Dependencies

Edit `src-tauri/Cargo.toml`:

```toml
[dependencies]
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.11", features = ["json"] }
tokio = { version = "1", features = ["full"] }
```

## Platform-Specific Code

```rust
#[cfg(target_os = "windows")]
fn platform_specific() {
    // Windows-only code
}

#[cfg(target_os = "macos")]
fn platform_specific() {
    // macOS-only code
}

#[cfg(target_os = "linux")]
fn platform_specific() {
    // Linux-only code
}

#[cfg(target_os = "android")]
fn platform_specific() {
    // Android-only code
}

#[cfg(target_os = "ios")]
fn platform_specific() {
    // iOS-only code
}
```
