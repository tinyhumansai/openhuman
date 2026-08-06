//! Utility functions for `OpenHuman`.
//!
//! Kernel family — always compiled, never gated. These are dependency-free
//! helpers reused across domains; nothing here may reach into a domain.
//!
//! - [`text`]     — UTF-8-safe truncation, char-boundary rounding, provenance tags
//! - [`retry`]    — retry-with-backoff + transient-filesystem-error classification
//! - [`sanitize`] — LLM-facing text sanitization (control-char stripping,
//!   instruction-fence removal, UTF-8-safe byte caps)
//! - [`tls`]      — TLS client/connector construction
//! - [`types`]    — shared utility types
//!
//! Everything is re-exported at the module root, so the pre-reorg
//! `openhuman::util::<fn>` paths (including the `truncate_with_ellipsis`
//! doctest) still resolve.

pub mod retry;
pub mod sanitize;
pub mod text;
pub mod tls;
pub mod types;

pub use retry::{is_transient_fs_error, retry_with_backoff, retry_with_backoff_async};
pub use text::{
    ceil_char_boundary, floor_char_boundary, provenance_tag, truncate_at_byte_boundary,
    truncate_with_ellipsis, truncate_with_suffix, utf8_safe_prefix_at_byte_boundary,
};
pub use types::MaybeSet;
