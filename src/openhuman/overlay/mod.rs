//! Overlay process launcher — discovers and spawns the `openhuman-overlay`
//! Tauri application as a child process so the floating debug/voice panel
//! appears automatically when the core RPC server is running.

mod process;

pub use process::spawn_overlay;
