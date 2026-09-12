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
//!   - `composio.sync`                → `openhuman.composio_sync`

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
include!("schemas_part_01.rs");
include!("schemas_part_02.rs");
include!("schemas_part_03.rs");
