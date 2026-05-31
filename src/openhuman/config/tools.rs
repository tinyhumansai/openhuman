//! LLM-callable wrappers over the `config` domain (reads only).
//!
//! Config reads (snapshot, client config, autonomy, search, runtime flags,
//! resolved API URL, data paths) are default-ON and let the agent explain how
//! it is configured.
//!
//! The `config_update_*` mutators are intentionally NOT exposed here yet: the
//! domain's apply functions take hand-built `*SettingsPatch` structs (which are
//! not `Deserialize`) mapped field-by-field from separate wire types in
//! `config::schemas`. Exposing them as agent tools needs either those wire
//! types made public or a small `config::ops` patch-from-json helper — tracked
//! as a follow-up. They would all ship default-OFF (and autonomy is privilege
//! escalation), so leaving them out keeps this PR's surface read-only and safe.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{Tool, ToolResult};

use super::ops;

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// Read tool over an arg-less `async fn() -> Result<RpcOutcome<Value>, String>`.
macro_rules! read_tool {
    ($ty:ident, $name:literal, $fn:ident, $desc:literal) => {
        pub struct $ty;
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn parameters_schema(&self) -> serde_json::Value {
                json!({ "type": "object", "properties": {} })
            }
            async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
                log::debug!(concat!("[tool][config] ", $name, " invoked"));
                emit!(ops::$fn().await, $name)
            }
            fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
                true
            }
        }
    };
}

/// Full config snapshot (needs the in-scope config).
pub struct ConfigSnapshotTool {
    config: Arc<Config>,
}

impl ConfigSnapshotTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ConfigSnapshotTool {
    fn name(&self) -> &str {
        "config_snapshot"
    }

    fn description(&self) -> &str {
        "Return a full snapshot of the effective core configuration. Use to \
         inspect how the assistant is configured."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][config] snapshot invoked");
        emit!(
            ops::get_config_snapshot(&self.config).await,
            "config_snapshot"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Runtime flags (sync, returns RpcOutcome directly).
pub struct ConfigRuntimeFlagsTool;

#[async_trait]
impl Tool for ConfigRuntimeFlagsTool {
    fn name(&self) -> &str {
        "config_get_runtime_flags"
    }

    fn description(&self) -> &str {
        "Return the effective runtime feature flags (env-derived toggles)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][config] runtime_flags invoked");
        let outcome = ops::get_runtime_flags();
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

read_tool!(
    ConfigClientConfigTool,
    "config_get_client_config",
    load_and_get_client_config_snapshot,
    "Return the redacted client-facing config snapshot."
);
read_tool!(
    ConfigAutonomyTool,
    "config_get_autonomy",
    get_autonomy_settings,
    "Return the current agent autonomy/access settings."
);
read_tool!(
    ConfigSearchTool,
    "config_get_search",
    get_search_settings,
    "Return the current search settings (API keys redacted)."
);
read_tool!(
    ConfigResolveApiUrlTool,
    "config_resolve_api_url",
    load_and_resolve_api_url,
    "Return the effective backend API URL after resolution."
);
read_tool!(
    ConfigDataPathsTool,
    "config_get_data_paths",
    get_data_paths,
    "Return the resolved on-disk data directories and workspace marker."
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

    #[test]
    fn read_metadata() {
        assert_eq!(
            ConfigSnapshotTool::new(Arc::new(Config::default())).name(),
            "config_snapshot"
        );
        assert_eq!(ConfigAutonomyTool.name(), "config_get_autonomy");
        assert_eq!(
            ConfigAutonomyTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(ConfigRuntimeFlagsTool.scope(), ToolScope::All);
    }
}
