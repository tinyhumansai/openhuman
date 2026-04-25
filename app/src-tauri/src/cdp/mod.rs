//! Shared Chrome DevTools Protocol client for the CEF-backed scanners.
//!
//! Consolidates the CdpConn / target-discovery / UA-override plumbing that
//! used to be copy-pasted across `discord_scanner`, `whatsapp_scanner`,
//! `slack_scanner`, and `telegram_scanner`. Scanners now call helpers here
//! instead of maintaining their own WebSocket dispatch.
//!
//! All CDP work is CEF-only — wry has no remote-debugging port. This module
//! is only compiled under `feature = "cef"` (see `lib.rs`).

pub mod conn;
pub mod emulation;
pub mod session;
pub mod snapshot;
pub mod target;

pub use conn::CdpConn;
pub use emulation::{set_user_agent_override, UaSpec};
pub use session::{
    placeholder_marker, placeholder_url, spawn_session, target_url_fragment, SpawnedSession,
};
pub use snapshot::Snapshot;
pub use target::{
    browser_ws_url, connect_and_attach_matching, detach_session, find_page_target_where,
};

/// Remote debugging host — matches `--remote-debugging-port=9222` in
/// `lib.rs`. Kept as constants so scanners and the session opener agree.
pub const CDP_HOST: &str = "127.0.0.1";
pub const CDP_PORT: u16 = 9222;
