//! Host-owned provider OAuth flows.
//!
//! This module owns the provider protocol and short-lived PKCE state. The core
//! receives completed token sets and persists them in its credential store.

mod controllers;
mod pending;
mod source;

pub(crate) use controllers::extension;
pub(crate) use source::OpenAiProviderCredentialSource;

#[cfg(test)]
#[path = "controllers_tests.rs"]
mod controllers_tests;

#[cfg(test)]
#[path = "source_tests.rs"]
mod source_tests;
