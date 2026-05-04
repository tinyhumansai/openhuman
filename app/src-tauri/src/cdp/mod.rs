//! Shared Chrome DevTools Protocol client for the CEF-backed scanners.
//!
//! Consolidates the CdpConn / target-discovery / notification-shim plumbing
//! that used to be copy-pasted across `discord_scanner`, `whatsapp_scanner`,
//! `slack_scanner`, and `telegram_scanner`. Scanners now call helpers here
//! instead of maintaining their own WebSocket dispatch.

pub mod conn;
pub mod input;
pub mod session;
pub mod snapshot;
pub mod target;

pub use conn::CdpConn;
pub use session::{
    placeholder_marker, placeholder_url, spawn_session, target_url_fragment, SpawnedSession,
};
#[allow(unused_imports)] // `Rect` re-export consumed once turn 2 lands; keep stable.
pub use snapshot::{Rect, Snapshot};
pub use target::{
    browser_ws_url, connect_and_attach_matching, detach_session, find_page_target_where,
};

/// Remote debugging host — matches `--remote-debugging-port=19222` in
/// `lib.rs`. Kept as constants so scanners and the session opener
/// agree. Port was 9222 originally but collided with ollama's
/// `127.0.0.1:9222` listener (silent CDP-attach failure → blank
/// child webviews). If you change either constant, update both.
pub const CDP_HOST: &str = "127.0.0.1";
pub const CDP_PORT: u16 = 19222;
