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
//! `tinyagents_harness::tool::schema_walk`.

mod contract;
mod lookups;
mod probe;

#[cfg(test)]
#[path = "catalog_in_flight_tests_tests.rs"]
mod in_flight_tests;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

pub(crate) use contract::fetch_live_toolkit_catalog;
#[cfg(test)]
pub(crate) use contract::seed_live_catalog_cache;
#[cfg(all(test, feature = "flows"))]
pub(crate) use contract::seed_live_catalog_cache_expired;
pub use contract::ToolContract;
#[cfg(any(test, feature = "flows"))]
pub(crate) use lookups::composio_required_args;
#[cfg(test)]
pub(crate) use probe::ProbedOutputSample;
#[cfg(any(test, feature = "flows"))]
pub(crate) use probe::{apply_probe_override, probe_tool_output_sample};
#[cfg(test)]
pub(crate) use probe::{seed_probe_cache, seed_probe_cache_expired};

// Brought into this module's own namespace (private `use`, not `pub use`) so
// `catalog_tests.rs` / `catalog_in_flight_tests_tests.rs` — declared as
// direct child modules of `catalog` above — can still reach the
// implementation details they exercise via a plain `use super::*;`, exactly
// as when this was one un-split file. See each item's `pub(super)` in its
// owning submodule.
#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use contract::{compute_composio_array_path, live_catalog_fetch_lock};
#[cfg(test)]
use lookups::composio_response_fields;
#[cfg(test)]
use probe::{
    cache_probe_result, probed_output_sample, resolve_composio_action_scope,
    COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT,
};
#[cfg(test)]
use serde_json::Value;
