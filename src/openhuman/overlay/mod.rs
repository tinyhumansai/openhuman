//! Overlay domain — signals pushed to the desktop overlay window.
//!
//! The Tauri desktop shell hosts a separate `overlay` window (see
//! `app/src-tauri/tauri.conf.json`) that renders `OverlayApp.tsx`. Because
//! the overlay runs in its own WebView with its own JS runtime, it cannot
//! share Redux state with the main window. Instead it subscribes to a
//! dedicated Socket.IO connection against the core process (same pattern
//! `useDictationHotkey` uses) and reacts to events emitted here.
//!
//! Currently the overlay activates in two cases:
//!   1. **STT / dictation** — driven by the existing `dictation:toggle`
//!      and `dictation:transcription` events (see `voice::dictation_listener`).
//!   2. **Attention** — a short, user-visible message the core wants to
//!      surface without stealing focus. Any core-side caller (subconscious
//!      loop, heartbeat, screen intelligence, …) can publish an
//!      `OverlayAttentionEvent` via [`publish_attention`] and it will be
//!      broadcast to the overlay window as `overlay:attention`.
//!
//! Keep this module light: it is export-focused and owns one broadcast
//! bus. The Socket.IO bridge lives in `src/core/socketio.rs`.

pub mod bus;
pub mod types;

pub use bus::{publish_attention, subscribe_attention_events};
pub use types::{OverlayAttentionEvent, OverlayAttentionTone};
