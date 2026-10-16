//! Journal export readiness: whether a run's durable tinyagents observations
//! would actually be sent to the backend's Langfuse proxy for a given config.

use crate::config::Config;
use crate::security::credentials::session_support::direct_backend_credential;

use super::{environment_for_base, otlp_url, skip_push};

/// Whether a journal push would actually send for `config`: the URL gate, the
/// environment gate and a live session credential. Callers must read a journal
/// and build observations first, so without a live session (unit tests, a
/// signed-out or embedder host) or on a skipped environment that work is
/// discarded anyway, and reading a whole child journal is not free.
pub(crate) fn journal_push_ready(config: &Config) -> bool {
    let url = otlp_url(config);
    !skip_push(environment_for_base(config, &url))
        && url.starts_with("http")
        && matches!(
            direct_backend_credential(config, "langfuse journal push"),
            Some(crate::security::credentials::session_support::BackendCredential::Session(_))
        )
}
