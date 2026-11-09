//! Typed layout facade for the physical legacy SQLite inventory and copy.
//!
//! All SQLite access to legacy databases lives under `storage::import`, behind
//! the `legacy-import` feature. Layout migration code depends on this narrow
//! wrapper and does not open or query SQLite itself.

pub use crate::storage::import::layout_db::{checkpoint, copy, table_counts, Counts, DbCopy};
