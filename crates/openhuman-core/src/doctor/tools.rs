//! LLM-callable wrappers over the `doctor` diagnostics domain.
//!
//! Read-only health diagnostics; both tools delegate to
//! [`crate::doctor::ops`] and ship default-ON.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::config::Config;
use tinytools::{Tool, ToolResult};

use super::ops;
use super::{DoctorOptions, DoctorTier};

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// Run the full diagnostic battery.
pub struct DoctorHealthTool {
    config: Arc<Config>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoctorHealthArgs {
    tier: Option<DoctorTier>,
    only: Option<Vec<String>>,
}

impl DoctorHealthTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DoctorHealthTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "doctor_health"
    }

    fn description(&self) -> &str {
        "Run selected OpenHuman health checks and return a structured v2 report. \
         The default connect tier uses bounded, read-only checks. The live tier \
         can spend inference tokens and write disposable test data; run live only \
         when the user explicitly requested those billable probes. Use exact \
         registered check ids with `only` to narrow the report."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "tier": {
                    "type": "string",
                    "enum": ["static", "connect", "live"],
                    "default": "connect",
                    "description": "Maximum check tier. Live can spend inference tokens and write disposable test data; select it only when the user explicitly requested these billable probes."
                },
                "only": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional exact registered check ids to run. Omit to include all checks up to the selected tier."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let args = if args.is_null() {
            DoctorHealthArgs::default()
        } else {
            serde_json::from_value::<DoctorHealthArgs>(args)?
        };
        let tier = args.tier.unwrap_or(DoctorTier::Connect);
        let only = args.only.unwrap_or_default();
        log::debug!("[tool][doctor] health invoked");
        emit!(
            ops::doctor_report_with_options(
                &self.config,
                DoctorOptions {
                    tier,
                    only,
                    ..DoctorOptions::default()
                }
            )
            .await,
            "doctor_health"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Probe model availability.
pub struct DoctorModelsTool {
    config: Arc<Config>,
}

impl DoctorModelsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DoctorModelsTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "doctor_models"
    }

    fn description(&self) -> &str {
        "Check configured inference providers by listing their models catalogs. \
         This read-only check never generates a model response."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "use_cache": { "type": "boolean", "description": "Reuse a recent catalog result when available." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][doctor] models invoked");
        let use_cache = args
            .get("use_cache")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        emit!(
            ops::doctor_models(&self.config, use_cache).await,
            "doctor_models"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Build this domain's tools in their persisted catalogue order.
pub(crate) fn registered_tools(
    config: std::sync::Arc<crate::config::Config>,
) -> Vec<Box<dyn tinytools::Tool>> {
    vec![
        Box::new(DoctorHealthTool::new(config.clone())),
        Box::new(DoctorModelsTool::new(config.clone())),
    ]
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
