//! Live Composio tool contracts: the catalog, the probe, and their caches.
//!
//! What a Composio action really accepts and really returns, sourced from
//! Composio itself rather than from a static curated list — the ground truth the
//! workflow builder authors against, and the enforcement gates validate against.
//!
//! Two sources, in priority order:
//!
//! 1. **The published schema.** [`fetch_live_toolkit_catalog`] reads Composio's
//!    own v3 `/tools` listing and derives a [`ToolContract`] per action.
//! 2. **A real response.** Many actions publish no `output_parameters` at all,
//!    so [`probe_tool_output_sample`] makes one bounded, READ-only, real call
//!    and derives the same hints from the actual value.
//!    [`apply_probe_override`] lets a probe win over a schema, because an
//!    observed response outranks a documented one.
//!
//! # Why this lives in `composio`, not in the workflow adapter seam
//!
//! It used to live in the seam, which put the dependency backwards: an
//! always-compiled domain reaching into a feature-gated adapter. That made the
//! adapter impossible to gate off, since `composio` would have followed it out
//! of the build. Everything here is Composio's own vocabulary — action slugs,
//! toolkits, the execute-response envelope, connection scope — so it belongs to
//! the domain that owns that vocabulary. The seam now imports from here.
//!
//! The vendor-neutral half of the work (walking a JSON Schema or a JSON value
//! for its primary array and its field names) is in
//! [`crate::openhuman::json_schema`], owned by neither side.

#[cfg(test)]
#[path = "catalog_in_flight_tests_tests.rs"]
mod in_flight_tests;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
include!("catalog_part_01.rs");
include!("catalog_part_02.rs");
