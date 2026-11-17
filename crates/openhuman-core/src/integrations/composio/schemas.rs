//! Controller schemas + registered handlers for the Composio domain.
//!
//! Exposes the domain over the shared registry at
//! `openhuman.composio_*`:
//!   - `composio.list_toolkits`       → `openhuman.composio_list_toolkits`
//!   - `composio.list_capabilities`   → `openhuman.composio_list_capabilities`
//!   - `composio.list_agent_ready_toolkits` → `openhuman.composio_list_agent_ready_toolkits`
//!   - `composio.list_connections`    → `openhuman.composio_list_connections`
//!   - `composio.authorize`           → `openhuman.composio_authorize`
//!   - `composio.delete_connection`   → `openhuman.composio_delete_connection`
//!   - `composio.list_tools`          → `openhuman.composio_list_tools`
//!   - `composio.execute`             → `openhuman.composio_execute`
//!   - `composio.list_github_repos`   → `openhuman.composio_list_github_repos`
//!   - `composio.create_trigger`      → `openhuman.composio_create_trigger`
//!   - `composio.get_user_profile`    → `openhuman.composio_get_user_profile`
//!   - `composio.refresh_all_identities` → `openhuman.composio_refresh_all_identities`

mod definitions;
mod handlers_connections;
mod handlers_identity;
mod handlers_tools;
mod handlers_triggers;
mod params;
mod registry;
mod util;

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;

pub(crate) use registry::all_controller_entries;
pub use registry::{all_controller_schemas, all_registered_controllers};

pub use definitions::schemas;

// Brought into this module's own namespace (private `use`, not `pub use`)
// so `schemas_tests.rs` — declared as a direct child module of `schemas`
// above — can still reach these via a plain `use super::*;`, exactly as
// when this was one un-split file. See each item's `pub(super)` in its
// owning submodule.
#[cfg(test)]
use serde_json::{Map, Value};
#[cfg(test)]
use util::{read_optional, read_required, read_required_non_empty};
