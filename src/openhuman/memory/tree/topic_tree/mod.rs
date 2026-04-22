//! Phase 3c — topic trees (lazy materialisation) (#709).
//!
//! A *topic tree* is a per-entity summary tree whose leaves are all
//! chunks mentioning that entity, regardless of the source they came
//! from. Topic trees are spawned lazily — only when an entity's hotness
//! crosses a threshold — and then receive new leaves via the ingest path
//! alongside the per-source tree. See `docs/MEMORY_ARCHITECTURE_LLD.md`
//! for the full design.
//!
//! Phase 3c surface:
//! - [`curator::maybe_spawn_topic_tree`] — per-ingest tick; bumps
//!   counters and spawns a topic tree when hotness crosses
//!   [`types::TOPIC_CREATION_THRESHOLD`].
//! - [`routing::route_leaf_to_topic_trees`] — called by the ingest path
//!   after the source-tree append; fans a kept leaf out to every
//!   matching entity's topic tree.
//! - [`registry::get_or_create_topic_tree`] /
//!   [`registry::archive_topic_tree`] — primitives for admin flows.
//! - [`backfill::backfill_topic_tree`] — walk the entity index and
//!   hydrate a freshly spawned tree with historic leaves.
//! - [`hotness::hotness`] — pure arithmetic over pre-existing signals;
//!   easy to unit-test.
//!
//! Tree mechanics (buffer, seal, cascade) are **not reimplemented** here
//! — `append_leaf` from [`super::source_tree::bucket_seal`] takes a
//! `&Tree` so it works for any `TreeKind`. The Phase 3c code only adds
//! the hotness layer and the per-entity fan-out.

pub mod backfill;
pub mod curator;
pub mod hotness;
pub mod registry;
pub mod routing;
pub mod store;
pub mod types;

pub use curator::{maybe_spawn_topic_tree, SpawnOutcome};
pub use hotness::{hotness, recency_decay};
pub use registry::{
    archive_topic_tree, force_create_topic_tree, get_or_create_topic_tree, list_topic_trees,
};
pub use routing::route_leaf_to_topic_trees;
pub use types::{
    EntityIndexStats, HotnessCounters, TOPIC_ARCHIVE_THRESHOLD, TOPIC_CREATION_THRESHOLD,
    TOPIC_RECHECK_EVERY,
};
