//! LLM-callable wrappers over the `service` (daemon lifecycle) domain.
//!
//! `service_status` and `daemon_host_prefs_get` are read-only and default-ON.
//! Every lifecycle mutator — start/stop/restart/shutdown/install/uninstall and
//! the tray-prefs setter — changes the running process or the installed system
//! service, so they ship default-OFF via `tools/user_filter.rs`
//! (`service_lifecycle` toggle). shutdown/install/uninstall are `Dangerous`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops;

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

fn opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Macro to define a simple `&Config`-only service tool.
macro_rules! config_tool {
    ($ty:ident, $name:literal, $fn:ident, $perm:expr, $desc:literal) => {
        pub struct $ty {
            config: Arc<Config>,
        }
        impl $ty {
            pub fn new(config: Arc<Config>) -> Self {
                Self { config }
            }
        }
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
            fn permission_level(&self) -> PermissionLevel {
                $perm
            }
            async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
                log::debug!(concat!("[tool][service] ", $name, " invoked"));
                emit!(ops::$fn(&self.config).await, $name)
            }
        }
    };
}

config_tool!(
    ServiceStatusTool,
    "service_status",
    service_status,
    PermissionLevel::ReadOnly,
    "Report the daemon/service status (installed, running, version)."
);
config_tool!(
    DaemonHostPrefsGetTool,
    "daemon_host_prefs_get",
    daemon_host_get,
    PermissionLevel::ReadOnly,
    "Read the daemon host UI preferences (e.g. show-tray flag)."
);
config_tool!(
    ServiceStartTool,
    "service_start",
    service_start,
    PermissionLevel::Execute,
    "Start the OpenHuman daemon service. Default-OFF (opt-in)."
);
config_tool!(
    ServiceStopTool,
    "service_stop",
    service_stop,
    PermissionLevel::Execute,
    "Stop the OpenHuman daemon service. Default-OFF (opt-in)."
);
config_tool!(
    ServiceInstallTool,
    "service_install",
    service_install,
    PermissionLevel::Dangerous,
    "Install the OpenHuman daemon as a system service. Default-OFF (opt-in)."
);
config_tool!(
    ServiceUninstallTool,
    "service_uninstall",
    service_uninstall,
    PermissionLevel::Dangerous,
    "Remove the OpenHuman daemon system service. Default-OFF (opt-in)."
);

/// Set daemon host tray preference. Default-OFF.
pub struct DaemonHostPrefsSetTool {
    config: Arc<Config>,
}

impl DaemonHostPrefsSetTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DaemonHostPrefsSetTool {
    fn name(&self) -> &str {
        "daemon_host_prefs_set"
    }

    fn description(&self) -> &str {
        "Set the daemon host UI preference `show_tray`. Default-OFF (opt-in)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "show_tray": { "type": "boolean", "description": "Show the tray icon." } },
            "required": ["show_tray"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][service] daemon_host_prefs_set invoked");
        let show_tray = args
            .get("show_tray")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| anyhow::anyhow!("missing required boolean argument `show_tray`"))?;
        emit!(
            ops::daemon_host_set(&self.config, show_tray).await,
            "daemon_host_prefs_set"
        )
    }
}

/// Restart the core process. Default-OFF.
pub struct ServiceRestartTool;

#[async_trait]
impl Tool for ServiceRestartTool {
    fn name(&self) -> &str {
        "service_restart"
    }

    fn description(&self) -> &str {
        "Request an asynchronous restart of the core process, with optional \
         `source` and `reason` labels. Default-OFF (opt-in)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source": { "type": "string" },
                "reason": { "type": "string" }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][service] restart invoked");
        emit!(
            ops::service_restart(opt_str(&args, "source"), opt_str(&args, "reason")).await,
            "service_restart"
        )
    }
}

/// Gracefully shut down the core process. Default-OFF, Dangerous.
pub struct ServiceShutdownTool;

#[async_trait]
impl Tool for ServiceShutdownTool {
    fn name(&self) -> &str {
        "service_shutdown"
    }

    fn description(&self) -> &str {
        "Request a graceful shutdown of the core process, with optional \
         `source` and `reason` labels. Terminates the running assistant. \
         Default-OFF (opt-in)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source": { "type": "string" },
                "reason": { "type": "string" }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][service] shutdown invoked");
        emit!(
            ops::service_shutdown(opt_str(&args, "source"), opt_str(&args, "reason")).await,
            "service_shutdown"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn names_and_levels() {
        assert_eq!(ServiceStatusTool::new(cfg()).name(), "service_status");
        assert_eq!(
            ServiceStatusTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            ServiceStartTool::new(cfg()).permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            ServiceInstallTool::new(cfg()).permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(
            ServiceShutdownTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(
            ServiceRestartTool.permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            DaemonHostPrefsSetTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(ServiceStatusTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn daemon_host_set_requires_flag() {
        let err = DaemonHostPrefsSetTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing show_tray");
        assert!(err.to_string().contains("show_tray"));
    }
}
