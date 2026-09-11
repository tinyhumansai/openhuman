//! Host layer over the Composio provider vocabulary.
//!
//! This used to be `pub use tinymemory_core::sync::composio::providers::*;` —
//! a glob over the engine's entire provider registry (the six per-toolkit
//! providers, the `ComposioProvider` trait, `ProviderContext`, `sync_state`,
//! `profile`/`profile_md`, and the curated catalogs). tinymemory v1.13.4
//! deleted that whole tree: reaching a connected account now needs a
//! credential this crate must not hold, so there is no in-process registry to
//! glob-import any more (see `crate::openhuman::integrations::composio::providers`
//! for the fuller account of what replaced each piece).
//!
//! What is left here is `slack` — this host's own JSON-RPC surface for the
//! Composio-backed Slack provider (`openhuman.slack_memory_sync_trigger` /
//! `openhuman.slack_memory_sync_status`), which now reads and writes through
//! the `tinyconnectors` module rather than through an engine `SlackProvider`.

pub mod slack;
