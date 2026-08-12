//! Domain types for the Composio integration.
//!
//! The definitions **moved to `tinymemory_api::host::composio`** during the
//! memory extraction: the extracted sync pipelines read these fields directly
//! on every run, so they had to live somewhere both crates can name. They are
//! inert serde data and carry no dependencies, so the contract crate's
//! dependency-light guarantee is unaffected.
//!
//! Every existing `integrations::composio::types::…` path keeps resolving and
//! keeps naming the same types. **Their serde form mirrors the backend's
//! response envelopes** under `/agent-integrations/composio/*` — field names and
//! `#[serde(...)]` attributes are a wire contract, not an implementation
//! detail.

pub use tinymemory_api::host::composio::*;
