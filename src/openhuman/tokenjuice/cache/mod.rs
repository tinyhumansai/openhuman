//! CCR (Compress-Cache-Retrieve) — original storage + retrieval markers.

pub mod marker;
pub mod store;

pub use marker::{
    format_marker, is_recovery_tool, parse_markers, recovery_footer, LEGACY_RETRIEVE_TOOL_NAME,
    NEVER_COMPACT_TOOLS, RECOVERY_TOOL_NAMES, RETRIEVE_TOOL_NAME,
};
pub use store::{
    configure, disable_disk_tier, enable_disk_tier, offload, offload_checked, retrieve,
    retrieve_range, short_hash, stats, RangeUnit,
};
