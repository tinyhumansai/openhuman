//! Local AI runtime configuration.
//!
//! Definitions moved to [`tinymemory_api::host`] — the extracted memory
//! subsystem reads `local_ai` when it resolves an embedding backend. Re-exported
//! here so existing paths keep resolving.
//!
//! This is a host-owned section living in the memory contract crate for purely
//! mechanical reasons; moving embedding *construction* back into the host would
//! let it come home.

pub use tinymemory_api::host::local_ai::{LocalAiConfig, LocalAiUsage};
