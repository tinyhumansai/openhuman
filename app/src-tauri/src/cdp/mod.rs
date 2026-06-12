//! Shared Chrome DevTools Protocol client for the CEF-backed scanners.
//!
//! All CDP traffic flows through the in-process transport in
//! [`in_process`]: CDP messages travel directly between the Tauri shell
//! and the embedded CEF browser via `Webview::send_dev_tools_message`
//! and `Webview::on_dev_tools_protocol`. There is no listener and no
//! network surface; any same-UID process is shut out by construction.
//!
//! Scanners pick up a [`CdpConn`] either via [`conn_for_account`] (for
//! `acct_<id>`-labelled webviews) or [`conn_for_label`] /
//! [`connect_and_attach_matching_in_process_by_label`] (for other
//! surfaces such as the Meet call window).

pub mod conn;
pub mod in_process;
pub mod input;
pub mod session;
pub mod snapshot;
pub mod target;

pub use conn::CdpConn;
pub use in_process::{
    install_for_account, install_for_label, install_for_webview, set_cef_app_handle, CdpRegistry,
    EventFrame, WebviewCdpTransport, CALL_TIMEOUT,
};
pub use session::{
    placeholder_marker, placeholder_url, spawn_session, target_url_fragment, SpawnedSession,
};
#[allow(unused_imports)] // `Rect` re-export consumed once turn 2 lands; keep stable.
pub use snapshot::{Rect, Snapshot};
pub use target::{
    conn_for_account, conn_for_label, connect_and_attach_matching_in_process,
    connect_and_attach_matching_in_process_by_label, detach_session, find_page_target_where,
};
