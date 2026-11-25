//! Hosted observability: what this layer tells the core about its backend's
//! errors, and where usage-data traces go.
//!
//! [`register`] installs, in the core's observability slots:
//!
//! - the session-expiry predicate for backend path 401s
//!   ([`crate::session::expiry`]),
//! - the classifier for this layer's sentinels ([`crate::errors`]),
//! - the Langfuse usage-data trace sink ([`langfuse_sink`]).
//!
//! The channel-message Sentry filter registers with the channels area
//! (`crate::channels`).

pub(crate) mod langfuse_environment;
pub(crate) mod langfuse_sink;

use std::sync::Arc;

use openhuman_embed::__host::core::observability::register_trace_sink;

/// Installs the hosted classifiers and the Langfuse sink. Idempotent: every
/// registration replaces its id in place.
pub fn register() {
    log::debug!("[tinyhumans] observability: registering hosted classifiers and trace sink");
    crate::session::expiry::register();
    crate::errors::register();
    register_trace_sink(
        langfuse_sink::SINK_ID,
        Arc::new(langfuse_sink::LangfuseSink),
    );
}
