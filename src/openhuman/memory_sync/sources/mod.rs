//! Memory-source sync pipelines.
//!
//! Syncs user-added memory sources (GitHub repos, local folders, RSS feeds,
//! web pages) by pulling items through `memory_sources::readers` and landing
//! them directly into the memory tree as leaves. When a tree's L0 buffer
//! hits `INPUT_TOKEN_BUDGET` (50k tokens), the cascade sealer fires.
//!
//! Embeddings are temporarily disabled — leaves land without vectors and
//! the seal cascade uses `LabelStrategy::Empty`.

pub mod audit;
pub mod github;
pub mod rebuild;
