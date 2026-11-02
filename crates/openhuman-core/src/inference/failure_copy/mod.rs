//! One host copy table for user-facing failure text.
//!
//! - [`table`]: failure class -> (`error_type` wire token, `source`,
//!   `retryable`, user copy), consumed by `web_chat::web_errors` to build the
//!   `chat_error` envelope.
//! - `localized`: renders that copy in the channel's configured locale for
//!   native channel replies (`render_channel_failure`).
//!
//! Provider-neutral and host-specific error copy used by the chat error surface.

mod chat_error_catalog;
mod localized;
mod table;

pub(crate) use localized::{render_channel_failure, supported_channel_locale};
pub(crate) use table::{failure_copy, FailureClass};

#[cfg(test)]
#[path = "localized_tests.rs"]
mod localized_tests;
