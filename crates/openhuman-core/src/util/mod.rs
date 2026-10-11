//! Utility functions for `OpenHuman`.
//!
//! Kernel family — always compiled, never gated. Self-contained helpers
//! reused across domains; nothing here may reach into a domain (no `use
//! crate::` anywhere in this directory).
//!
//! - [`text`]     — UTF-8-safe truncation, char-boundary rounding, provenance tags
//! - [`retry`]    — retry-with-backoff + transient-filesystem-error classification
//! - `tinytools_std::sanitize` — LLM-facing text sanitization (control-char stripping,
//!   instruction-fence removal, UTF-8-safe byte caps)
//! - [`tls`]      — platform-conditional `reqwest::ClientBuilder` factory
//! - [`types`]    — shared utility types
//!
//! `retry`, `text`, and `types` items are re-exported at the module root, so
//! `crate::util::<fn>` (and `openhuman_core::util::<fn>` from outside, the
//! path the `truncate_with_ellipsis` doctest uses) resolves without naming the
//! submodule. `redact`, `sanitize`, and `tls` are reached through
//! their submodule path.
//!
//! See [`README.md`](README.md) for a per-file breakdown.

/// Generic JSON-RPC param deserialisation (`read_required`/`read_optional`),
/// shared across domain `schemas.rs` files.
pub mod params;
/// PII redaction for log output. See the module docs for why this is here and
/// not taken from the memory engine.
pub mod redact;
pub mod retry;
pub mod text;
pub mod tls;
pub mod types;
pub mod url;

pub use params::{read_optional, read_required};
pub use redact::redact_url_for_log;
pub use retry::{is_transient_fs_error, retry_with_backoff, retry_with_backoff_async};
pub use text::{
    ceil_char_boundary, floor_char_boundary, truncate_at_byte_boundary, truncate_with_ellipsis,
    truncate_with_suffix, utf8_safe_prefix_at_byte_boundary,
};
pub use types::MaybeSet;

#[cfg(test)]
pub(crate) mod module_test_process;
