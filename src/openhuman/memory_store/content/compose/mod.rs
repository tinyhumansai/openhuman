//! YAML front-matter + body composition for chunk `.md` files.
//!
//! Each file written to disk has the form:
//! ```text
//! ---
//! source_kind: chat
//! source_id: slack:#eng
//! seq: 0
//! owner: alice@example.com
//! timestamp: 2026-04-28T10:00:00Z
//! time_range_start: 2026-04-28T10:00:00Z
//! time_range_end: 2026-04-28T10:05:00Z
//! source_ref: slack://permalink/…
//! tags:
//!   - person/Alice-Smith
//!   - project/Phoenix
//! ---
//! ## 2026-04-28T10:00:00Z — alice
//! Message body here.
//! ```
//!
//! For email source_kind, additional fields are emitted:
//! ```text
//! participants:
//!   - alice@example.com
//!   - bob@example.com
//! aliases:
//!   - "alice@example.com <-> bob@example.com: chunk 0"
//! ```
//! These are parsed from the `source_id` field (format `gmail:{participants}`
//! where `participants` is `addr1|addr2|...` pipe-separated) at compose time.
//! `sender` and `thread_id` are no longer emitted — they are not meaningful
//! with participant-based bucketing.
//!
//! **SHA-256 is computed over the body bytes only** (everything after `---\n`
//! on the second delimiter line). This allows tags to be rewritten atomically
//! without invalidating the content hash.

pub mod chunk;
pub mod summary;
pub mod yaml;

#[cfg(test)]
mod tests;

pub const MEMORY_ARTIFACT_FORMAT: u32 = 2;
pub const OPENHUMAN_CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Re-exports (preserve original public API) ────────────────────────────────

pub use chunk::{compose_chunk_file, rewrite_tags};
pub use summary::{compose_summary_md, rewrite_summary_tags, ComposedSummary, SummaryComposeInput};
pub use yaml::{scan_fm_field, source_tag, split_front_matter, with_source_tag};
