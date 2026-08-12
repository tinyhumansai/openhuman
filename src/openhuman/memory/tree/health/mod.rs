//! Host layer over [`tinymemory_core::tree::health`].
//!
//! The health model and its classifier are core; what stays here is the
//! `user_error` wire payload, which rides a web channel.

pub use tinymemory_core::tree::health::*;

pub(crate) mod user_error;
