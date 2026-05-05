# Tree global

Phase 3b (#709) — Global Activity Digest tree. One singleton tree per workspace whose L0 nodes are end-of-day digests folded across every active source tree, sealing upward into weekly (L1), monthly (L2), and yearly (L3) recaps. Reuses Phase 3a storage (`mem_tree_trees` / `mem_tree_summaries` / `mem_tree_buffers` with `kind='global'`) and the `Summariser` trait, but uses a **count-based** seal trigger aligned to the time axis instead of the source tree's token-budget gate.

## Public surface

- `pub fn get_or_create_global_tree` — `registry.rs` — singleton lookup keyed on `(kind=global, scope="global")`.
- `pub fn end_of_day_digest` / `pub enum DigestOutcome` — `digest.rs` — build one L0 daily node from cross-source material and cascade-seal upward.
- `pub fn append_daily_and_cascade` — `seal.rs` — append a daily summary id into the L0 buffer and run the count-based cascade.
- `pub fn recap` / `pub fn pick_level` / `pub struct RecapOutput` — `recap.rs` — pick the right level for a window duration and assemble the recap.
- `pub const WEEKLY_SEAL_THRESHOLD` / `pub const MONTHLY_SEAL_THRESHOLD` / `pub const YEARLY_SEAL_THRESHOLD` / `pub const GLOBAL_SCOPE` / `pub const GLOBAL_TOKEN_BUDGET` — `mod.rs`.

## Files

- `mod.rs` — module surface, threshold constants, scope literal.
- `registry.rs` — get-or-create for the singleton global tree.
- `digest.rs` — end-of-day digest builder; idempotent on re-runs for the same calendar day.
- `seal.rs` — count-based cascade seal (7 daily → 1 weekly → 1 monthly → 1 yearly).
- `recap.rs` — read-side level picker plus fallback when higher levels haven't sealed yet.
- `digest_tests.rs` — unit tests for the digest builder, included via `#[path]`.
