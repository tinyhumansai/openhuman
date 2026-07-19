//! Telegram channel — host-side glue.
//!
//! The transport (Bot API driver, session store, pairing) now lives in
//! `tinychannels::providers::telegram`; re-export it here so existing paths
//! keep resolving. What stays are the host-coupled pieces that depend on the
//! OpenHuman event bus, runtime context, and cross-channel calls:
//! - [`remote_control`] — `/status /sessions /new` command handling (uses the
//!   agent runtime context + web session invalidation).
//! - [`bus`] — the `TelegramRemoteSubscriber` busy-state event handler.
//! - [`approval_surface`] — the `TelegramApprovalSurfaceSubscriber`.

mod approval_surface;
mod bus;
pub mod remote_control;

// Transport moved to tinychannels.
pub use tinychannels::providers::telegram::{session_store, TelegramChannel};

pub use approval_surface::{TelegramApprovalSurfaceSubscriber, TELEGRAM_APPROVAL_CLIENT_ID};
pub use bus::TelegramRemoteSubscriber;
pub use remote_control::TelegramRemoteCommand;

#[cfg(any(test, debug_assertions))]
pub mod test_support {
    //! Debug-build seams for raw integration coverage of Telegram send helpers.
    //! Delegates to the tinychannels transport crate where the logic now lives.

    pub use tinychannels::providers::telegram::test_support::parse_reaction_marker_for_test;
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod bus_tests;
