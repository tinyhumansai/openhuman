# Tree topic

Phase 3c (#709) — per-entity topic trees with lazy materialisation. A topic tree groups every chunk mentioning one canonical entity, regardless of source. Trees are spawned only when an entity's hotness crosses `TOPIC_CREATION_THRESHOLD`; once spawned they receive new leaves via the ingest path alongside the per-source tree. Tree mechanics (buffer / seal / cascade) reuse `super::tree_source::bucket_seal` end-to-end — only the hotness layer and the per-entity fan-out live here.

## Public surface

- `pub fn maybe_spawn_topic_tree` / `pub fn force_recompute` / `pub enum SpawnOutcome` — `curator.rs` — bumps hotness counters per ingest and spawns + backfills on cadence.
- `pub fn route_leaf_to_topic_trees` — `routing.rs` — fan-out hook called by ingest after the source-tree append.
- `pub fn backfill_topic_tree` / `pub fn backfill_topic_tree_at` / `pub const BACKFILL_WINDOW_DAYS` — `backfill.rs` — hydrate a freshly spawned tree from the entity index.
- `pub fn get_or_create_topic_tree` / `pub fn force_create_topic_tree` / `pub fn list_topic_trees` / `pub fn archive_topic_tree` — `registry.rs`.
- `pub fn hotness` / `pub fn hotness_at` / `pub fn recency_decay` — `hotness.rs` — pure arithmetic over the entity stats.
- `pub struct EntityIndexStats` / `pub struct HotnessCounters` / `pub const TOPIC_CREATION_THRESHOLD` / `pub const TOPIC_ARCHIVE_THRESHOLD` / `pub const TOPIC_RECHECK_EVERY` — `types.rs`.
- `pub fn get` / `pub fn get_or_fresh` / `pub fn upsert` / `pub fn distinct_sources_for` / `pub fn count` — `store.rs` — `mem_tree_entity_hotness` persistence.

## Files

- `mod.rs` — module surface and re-exports.
- `types.rs` — `EntityIndexStats`, `HotnessCounters`, threshold / cadence constants.
- `hotness.rs` — pure hotness arithmetic; deterministic, unit-testable in isolation.
- `store.rs` — persistence for the per-entity counter row and `distinct_sources` aggregation.
- `curator.rs` — counter bumps, hotness recompute on cadence, spawn-and-backfill on first threshold crossing.
- `routing.rs` — per-leaf fan-out into matching active topic trees plus a curator tick.
- `registry.rs` — get-or-create / archive primitives for topic trees in `mem_tree_trees` (`kind='topic'`).
- `backfill.rs` — windowed (30 d) backfill from `mem_tree_entity_index` after spawn.
