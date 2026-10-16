//! Config loading, snapshotting, and core runtime-flag helpers.

mod load;
mod paths;
mod reset_local_data;
mod runtime_flags;
mod snapshot;

#[cfg(test)]
pub(crate) use load::normalize_loaded_config;
pub use load::{
    load_config_for_workspace_with_timeout, load_config_in, load_config_with_timeout,
    load_current_or_init, reload_config_from_paths, reload_config_snapshot_with_timeout,
};
pub(crate) use paths::fallback_workspace_dir;
#[cfg(test)]
pub(crate) use paths::{active_workspace_marker_path, config_openhuman_dir, default_openhuman_dir};
#[cfg(test)]
pub(crate) use reset_local_data::reset_local_data_for_paths;
#[cfg(all(test, windows))]
pub(crate) use reset_local_data::reset_local_data_remove_error;
pub use reset_local_data::{get_data_paths, get_data_paths_for_user, reset_local_data};
pub use runtime_flags::{
    agent_server_status, core_rpc_url_from_env, get_runtime_flags, set_browser_allow_all,
    RuntimeFlagsOut,
};
#[cfg(test)]
pub(crate) use runtime_flags::{
    env_flag_enabled, BROWSER_ALLOW_ALL_ENV, BROWSER_ALLOW_ALL_RPC_ENABLE_ENV,
};
pub use snapshot::{
    client_config_json, get_config_snapshot, get_dashboard_settings,
    load_and_get_client_config_snapshot, load_and_get_config_snapshot, snapshot_config_json,
};

#[cfg(test)]
pub(crate) use crate::config::Config;
#[cfg(test)]
pub(super) use load::seed_and_enrich_model_registry;

#[cfg(test)]
#[path = "loader_model_registry_seed_tests.rs"]
mod model_registry_seed_tests;

#[cfg(test)]
#[path = "loader_io_chain_tests.rs"]
mod loader_io_chain_tests;

#[cfg(test)]
#[path = "loader_current_tests.rs"]
mod loader_current_tests;

#[cfg(test)]
#[path = "loader_explicit_ctx_tests.rs"]
mod loader_explicit_ctx_tests;
