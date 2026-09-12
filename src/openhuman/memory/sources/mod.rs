//! Memory sources: the registry of connectors this workspace ingests from, the
//! readers that pull items out of them, and the JSON-RPC surface over both.
//!
//! # This module no longer globs the engine (#5560)
//!
//! It used to be `pub use tinymemory_core::sources::*;`, which made every
//! source read in the host a compile-time link to the memory engine. The
//! previous revision of these docs listed three things standing in the way, and
//! two of them are now done:
//!
//! 1. **The types had a home this crate did not depend on.** They are
//!    `tinymemory-sources`', an engine-neutral crate. It is a direct dependency
//!    now — it costs no crate this manifest did not already have (no
//!    `rusqlite`, no `tinycortex`), so the unlock really was the one
//!    `Cargo.toml` line the note predicted.
//! 2. **Two of the seven readers had no upstream twin.** `composio` and
//!    `twitter` were implemented in the engine crate but named nothing
//!    engine-shaped, so they came home unchanged. See [`readers`].
//! 3. **`sync` and `status` are wired into pieces that have not moved.** This
//!    one is settled too — see below. `reconcile` was on this line as well and
//!    came home first.
//!
//! ## How `sync` and `status` were resolved
//!
//! Neither was ported wholesale; each was split along the line between what the
//! host knows and what only a driver can answer.
//!
//! - [`sync`] reached `engine::run_source_pipeline`, `engine::{needs_rebuild,
//!   rebuild_tree_from_raw}`, `queue::store::retry_all_failed`,
//!   `sync::composio` and `sync::audit`. All of that stayed upstream, and none
//!   of it was reached from production here: `sync_source` has had no caller
//!   left in `src/` since the sync the product runs went over the bus through
//!   `MemorySourceSync`. The single live item, `derive_scopes`, is registry
//!   fields plus a scan of a directory this host owns, so it came home.
//! - [`status`] reached `store::chunks::store::with_connection`, the raw SQLite
//!   chunk door. That half is `MemoryChunks::source_ingest_status` now — the
//!   upstream ask this file used to record, a **pending** count per configured
//!   source, which `source_totals` still cannot answer (`SourceTotal` has no
//!   pending column and omits a source with zero chunks entirely). What stayed
//!   host-side is the half that was always the host's: the chunk-key prefix,
//!   derived from the registry entry, and the freshness label, which is
//!   arithmetic over a timestamp and this process's clock.
//!
//! What was *not* done in either case is porting the pipeline or the raw chunk
//! door into the host, which would be the opposite of what #5560 is for: a
//! second unpoliced door spelled differently is still a second unpoliced door.
//!
//! ## What came home, and why it was different
//!
//! [`reconcile`] was on the blocked list and no longer is. Both of its halves
//! read the `[[memory_sources]]` table in **this host's own config file** and
//! nothing below it: the scan came home when tinymemory v1.13.4 deleted the
//! in-process Composio pipeline it used to call, and
//! `apply_composio_source_caps_migration` followed in #5560. That is the line
//! between the two lists — a config rewrite is host work that happened to live
//! upstream, where an ingest pipeline and a SQLite cursor are not.
//!
//! `MemorySourceSink` is not the answer for the registry either — it is
//! `accept_source_items` + `forget_source` + `forget_matching`, an *ingest*
//! door with no listing or CRUD member for a configured connector, and it is
//! the whole of `Capability::Sources`. A listing member would be an upstream
//! ask; the registry did not need one, because the file it reads is this
//! host's own.

pub mod readers;
pub mod registry;
pub mod rpc;
pub mod schemas;

/// The source vocabulary, from the crate that defines it.
pub mod types {
    pub use tinymemory_sources::types::{
        ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind,
    };
}

pub use registry::{
    apply_kind_defaults, list_sources, memory_sync_defaults_for_toolkit, upsert_composio_source,
    ComposioUpsertTarget, MemorySourcePatch,
};
pub use types::{ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind};

// `status` and `sync` were the last two names this domain reached out of the
// engine crate for, and both are home now — see each module's own docs for
// which half moved and which half stayed upstream. The paths are unchanged
// (`sources::status::status_list`, `sources::sync::derive_scopes`), so no call
// site moved with them.
pub mod status;
pub mod sync;

// `reconcile` used to be entirely the engine's. tinymemory v1.13.4 deleted
// `ensure_composio_sources` along with the rest of the in-process Composio
// pipeline it scanned (`sync::composio::scan_active_sync_targets`), so this
// host carries its own — built on
// `memory::sync::composio::scan_active_sync_targets`, the tinyconnectors
// replacement. `apply_composio_source_caps_migration` followed it home in
// #5560: it never touched the deleted pipeline, only this host's config file,
// and reaching it through the engine bought a compile-time link to the engine
// for a `config.toml` rewrite.
pub mod reconcile;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_memory_sources_controller_schemas,
    all_registered_controllers as all_memory_sources_registered_controllers,
};
