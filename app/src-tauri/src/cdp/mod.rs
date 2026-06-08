//! Shared Chrome DevTools Protocol client for the CEF-backed scanners.
//!
//! Consolidates the CdpConn / target-discovery / notification-shim plumbing
//! that used to be copy-pasted across `discord_scanner`, `whatsapp_scanner`,
//! `slack_scanner`, and `telegram_scanner`. Scanners now call helpers here
//! instead of maintaining their own dispatch.
//!
//! # Transport
//!
//! The CDP transport is **in-process** — see [`in_process`]. CDP messages
//! travel directly between the Tauri shell and the embedded CEF browser
//! via `Webview::send_dev_tools_message` /
//! `Webview::on_dev_tools_protocol`. There is no TCP listener; the
//! previous `--remote-debugging-port=19222` flag was dropped because any
//! same-UID local process could attach to it without authentication.

pub mod conn;
pub mod in_process;
pub mod input;
pub mod session;
pub mod snapshot;
pub mod target;

pub use conn::CdpConn;
pub use in_process::{
    install_for_account, install_for_webview, set_cef_app_handle, CdpRegistry, EventFrame,
    WebviewCdpTransport, CALL_TIMEOUT,
};
pub use session::{
    placeholder_marker, placeholder_url, spawn_session, target_url_fragment, SpawnedSession,
};
#[allow(unused_imports)] // `Rect` re-export consumed once turn 2 lands; keep stable.
pub use snapshot::{Rect, Snapshot};
pub use target::{
    browser_ws_url, connect_and_attach_matching, conn_for_account, detach_session,
    find_page_target_where,
};

/// Remote debugging host — historical constant for scanner modules that
/// still use the TCP WebSocket path. The in-process transport in
/// [`in_process`] does not use these constants, but the per-scanner
/// `CdpConn` duplicates in `discord_scanner` / `whatsapp_scanner` /
/// `slack_scanner` / `telegram_scanner` still need them until they are
/// migrated to the shared in-process channel.
pub const CDP_HOST: &str = "127.0.0.1";
pub const CDP_PORT: u16 = 19222;
