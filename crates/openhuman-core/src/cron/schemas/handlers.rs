use crate::core::all::ControllerFuture;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tinyflows_bus::schedule::types::{CronJobPatch, DeliveryConfig};

pub(super) fn handle_add(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        if !config.cron.enabled {
            return Err("cron is disabled by config (cron.enabled=false)".to_string());
        }
        crate::core::runtime::components::require_config(&config, "tinyflows")?;

        let schedule: tinyflows_bus::schedule::types::Schedule =
            read_required(&params, "schedule")?;
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let session_target_str = params
            .get("session_target")
            .and_then(|v| v.as_str())
            .unwrap_or("isolated");
        let session_target = match session_target_str {
            "main" => tinyflows_bus::schedule::types::SessionTarget::Main,
            "isolated" => tinyflows_bus::schedule::types::SessionTarget::Isolated,
            "current" => tinyflows_bus::schedule::types::SessionTarget::Current,
            other => return Err(format!("invalid 'session_target': {other}")),
        };
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let delivery: Option<tinyflows_bus::schedule::types::DeliveryConfig> =
            match params.get("delivery") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    serde_json::from_value(v.clone())
                        .map_err(|e| format!("invalid 'delivery': {e}"))?,
                ),
            };
        let delete_after_run = params
            .get("delete_after_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let origin: Option<tinyflows_bus::schedule::types::JobOrigin> = match params.get("origin") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                serde_json::from_value(v.clone()).map_err(|e| format!("invalid 'origin': {e}"))?,
            ),
        };

        // Determine job type
        let job_type = match params.get("job_type").and_then(|v| v.as_str()) {
            Some("shell") => "shell",
            Some("agent") => "agent",
            Some(other) => return Err(format!("invalid 'job_type': {other}")),
            None => {
                if prompt.is_some() {
                    "agent"
                } else {
                    "shell"
                }
            }
        };

        let job = match job_type {
            "shell" => {
                let cmd = command.ok_or("'command' is required for shell jobs")?;
                let security = crate::security::SecurityPolicy::from_config(
                    &config.autonomy,
                    &config.workspace_dir,
                    &config.action_dir,
                );
                crate::cron::scheduler::shell_job::authorize_shell_command(
                    &config, &security, &cmd,
                )
                .await?;
                crate::cron::store::add_shell_job(&config, name, schedule, &cmd)
                    .map_err(|e| e.to_string())?
            }
            "agent" => {
                let p = prompt.ok_or("'prompt' is required for agent jobs")?;
                // RPC-created jobs default to enabled and, without an origin,
                // to no delivery (current behaviour).
                let delivery = match delivery {
                    Some(delivery) => Some(delivery),
                    None if origin.is_none() => Some(DeliveryConfig::default()),
                    None => Some(
                        crate::cron::client::default_delivery(&config, origin.as_ref())
                            .await
                            .map_err(|e| e.to_string())?,
                    ),
                };
                let delivery_cfg = delivery.clone().unwrap_or_default();
                // RPC `origin` is request data, not a verified turn context;
                // it cannot exempt a target from the configured allowlist.
                crate::cron::job_builder::validate_delivery_for_config_with_trusted_origin(
                    &config,
                    &delivery_cfg,
                    origin.as_ref(),
                    None,
                )?;
                crate::cron::client::check_origin_requirements(
                    &config,
                    &session_target,
                    &delivery_cfg,
                    origin.as_ref(),
                )
                .await
                .map_err(|e| e.to_string())?;
                let mut spec = tinyflows_bus::schedule::AgentJobSpec::new(schedule, p);
                spec.name = name;
                spec.session_target = session_target;
                spec.model = model;
                spec.delivery = delivery;
                spec.delete_after_run = delete_after_run;
                spec.agent_id = agent_id;
                spec.origin = origin;
                crate::cron::store::add_agent_job_from_spec(&config, spec)
                    .map_err(|e| e.to_string())?
            }
            other => return Err(format!("invalid 'job_type': {other}")),
        };

        to_json(Outcome::single_log(job, "cron job created"))
    })
}

pub(super) fn handle_list(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::cron::rpc::cron_list(&config).await?)
    })
}

pub(super) fn handle_update(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        let patch = read_required::<CronJobPatch>(&params, "patch")?;
        to_json(crate::cron::rpc::cron_update(&config, job_id.trim(), patch).await?)
    })
}

pub(super) fn handle_remove(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        to_json(crate::cron::rpc::cron_remove(&config, job_id.trim()).await?)
    })
}

pub(super) fn handle_run(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        to_json(crate::cron::rpc::cron_run(&ctx, &config, job_id.trim()).await?)
    })
}

pub(super) fn handle_runs(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        let limit = read_optional_u64(&params, "limit")?
            .map(|raw| usize::try_from(raw).map_err(|_| "limit is too large for usize".to_string()))
            .transpose()?;
        to_json(crate::cron::rpc::cron_runs(&config, job_id.trim(), limit).await?)
    })
}

pub(super) fn read_required<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

pub(super) fn read_optional_u64(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<u64>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(other) => Err(format!(
            "invalid '{key}': expected unsigned integer, got {}",
            type_name(other)
        )),
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

pub(super) fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
