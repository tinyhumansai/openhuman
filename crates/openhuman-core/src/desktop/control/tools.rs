//! Deferred desktop tools. Only their names and descriptions enter discovery.

use crate::tools::schema_cache::static_schema;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinycomputer_bus::{
    names, FindRequest, LaunchRequest, ListAppsRequest, ListWindowsRequest, RefRequest,
    RunGoalRequest, SnapshotRequest, TypeRequest, VisiblePredicate,
};
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolExposure, ToolResult, ToolRunContext, ToolTimeout,
};

use crate::config::Config;

#[derive(Clone, Copy)]
pub enum DesktopToolKind {
    Apps,
    Windows,
    Launch,
    Snapshot,
    Find,
    Act,
    Goal,
    ContinueGoal,
}

pub struct DesktopTool {
    config: Arc<Config>,
    kind: DesktopToolKind,
}

impl DesktopTool {
    pub fn new(config: Arc<Config>, kind: DesktopToolKind) -> Self {
        Self { config, kind }
    }
}

fn required(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: {key}"))
}

fn optional_string(args: &Value, key: &str) -> anyhow::Result<Option<String>> {
    args.get(key).map(|_| required(args, key)).transpose()
}

fn snapshot_request(args: &Value) -> anyhow::Result<SnapshotRequest> {
    Ok(SnapshotRequest {
        app: args.get("app").and_then(Value::as_str).map(str::to_owned),
        window_id: optional_string(args, "window_id")?,
        skeleton: args
            .get("skeleton")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        root_ref: args
            .get("root_ref")
            .and_then(Value::as_str)
            .map(str::to_owned),
        max_depth: args
            .get("max_depth")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 12) as u8),
        ..SnapshotRequest::default()
    })
}

fn goal_request(args: &Value, approvals_enabled: bool) -> anyhow::Result<Value> {
    let app = required(args, "app")?;
    let goal = required(args, "goal")?;
    let operations = args
        .get("allowed_operations")
        .and_then(Value::as_array)
        .filter(|operations| !operations.is_empty())
        .ok_or_else(|| anyhow::anyhow!("desktop_goal requires allowed_operations"))?;
    let targets = args
        .get("allowed_targets")
        .and_then(Value::as_array)
        .filter(|targets| {
            !targets.is_empty()
                && targets.iter().all(|target| {
                    target
                        .as_str()
                        .is_some_and(|name| !name.trim().is_empty())
                })
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "needs_inspection: supply allowed_targets using exact accessible names, descriptions, or native AX identifiers from desktop_snapshot or desktop_find"
            )
        })?;
    let success = args
        .get("success")
        .and_then(Value::as_array)
        .filter(|predicates| !predicates.is_empty())
        .ok_or_else(|| anyhow::anyhow!("desktop_goal requires a visible success predicate"))?;
    let mut request = json!({
        "app": app,
        "goal": goal,
        "include_values": false,
        "require_confirmations": approvals_enabled,
        "max_steps": args.get("max_steps").and_then(Value::as_u64).unwrap_or(12).clamp(1, 20),
        "max_model_calls": args.get("max_model_calls").and_then(Value::as_u64).unwrap_or(24).clamp(1, 40),
        "max_elapsed_ms": args.get("max_elapsed_ms").and_then(Value::as_u64).unwrap_or(120_000).clamp(1_000, 300_000),
        "success": success,
        "text_slots": args.get("text_slots").cloned().unwrap_or_else(|| json!({})),
        "allowed_operations": operations,
        "allowed_targets": targets,
    });
    if let Some(window) = args.get("window") {
        request["window"] = window.clone();
    }
    if let Some(window_id) = optional_string(args, "window_id")? {
        request["window_id"] = json!(window_id);
    }
    // Decode through the shared contract before dispatch, so malformed
    // predicates and operation names fail without reaching the native module.
    let request: RunGoalRequest = serde_json::from_value(request)?;
    if !request.success.iter().all(|predicate| match predicate {
        VisiblePredicate::NamePresent { name } => !name.trim().is_empty(),
        VisiblePredicate::NameContains { fragment, within } => {
            !fragment.trim().is_empty() && !within.trim().is_empty()
        }
        VisiblePredicate::ValueEquals { name, .. } => !name.trim().is_empty(),
        VisiblePredicate::ValueContains { name, value } => {
            !name.trim().is_empty() && !value.trim().is_empty()
        }
        VisiblePredicate::StateContains { name, state } => {
            !name.trim().is_empty() && !state.trim().is_empty()
        }
    }) {
        anyhow::bail!(
            "desktop_goal success predicates require nonblank names, state tokens, fragments, and ancestor scopes"
        );
    }
    if request
        .text_slots
        .keys()
        .any(|name| !request.allowed_targets.iter().any(|target| target == name))
    {
        anyhow::bail!("desktop_goal text_slots keys must match an exact allowed target");
    }
    Ok(serde_json::to_value(request)?)
}

#[async_trait]
impl Tool for DesktopTool {
    fn name(&self) -> &str {
        match self.kind {
            DesktopToolKind::Apps => "desktop_list_apps",
            DesktopToolKind::Windows => "desktop_list_windows",
            DesktopToolKind::Launch => "desktop_launch",
            DesktopToolKind::Snapshot => "desktop_snapshot",
            DesktopToolKind::Find => "desktop_find",
            DesktopToolKind::Act => "desktop_act",
            DesktopToolKind::Goal => "desktop_goal",
            DesktopToolKind::ContinueGoal => "desktop_continue_goal",
        }
    }

    fn description(&self) -> &str {
        match self.kind {
            DesktopToolKind::Apps => "List running desktop applications on this computer.",
            DesktopToolKind::Windows => "List native windows for a running desktop app.",
            DesktopToolKind::Launch => "Launch or activate a named desktop app so its window becomes available.",
            DesktopToolKind::Snapshot => "Inspect an app's accessibility tree and obtain snapshot-qualified element refs.",
            DesktopToolKind::Find => "Find a desktop accessibility element by role and name, returning a ref.",
            DesktopToolKind::Act => "Act on a desktop element ref: click, focus, type, check, uncheck, expand, or collapse.",
            DesktopToolKind::Goal => "Run one scoped, bounded Jev desktop task through its internal observe-act-verify loop. Supply a visible success predicate; discover via tool_search.",
            DesktopToolKind::ContinueGoal => "Resume a desktop goal after the user approved its exact pending action in Connections.",
        }
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn family(&self) -> Option<&str> {
        Some("desktop")
    }

    fn permission_level(&self) -> PermissionLevel {
        match self.kind {
            DesktopToolKind::Act
            | DesktopToolKind::Goal
            | DesktopToolKind::ContinueGoal
            | DesktopToolKind::Launch => PermissionLevel::Write,
            _ => PermissionLevel::ReadOnly,
        }
    }

    fn external_effect(&self) -> bool {
        matches!(
            self.kind,
            DesktopToolKind::Act
                | DesktopToolKind::Goal
                | DesktopToolKind::ContinueGoal
                | DesktopToolKind::Launch
        )
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        match self.kind {
            DesktopToolKind::Goal => {
                let budget_ms = args
                    .get("max_elapsed_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(120_000)
                    .clamp(1_000, 300_000);
                ToolTimeout::Millis(budget_ms + 30_000)
            }
            DesktopToolKind::ContinueGoal => ToolTimeout::Millis(330_000),
            _ => ToolTimeout::Inherit,
        }
    }

    fn parameters_schema(&self) -> Value {
        match self.kind {
            DesktopToolKind::Apps => static_schema!(include_str!("parameters/desktop_apps.json")),
            DesktopToolKind::Windows => {
                static_schema!(include_str!("parameters/desktop_windows.json"))
            }
            DesktopToolKind::Launch => {
                static_schema!(include_str!("parameters/desktop_launch.json"))
            }
            DesktopToolKind::Snapshot => {
                static_schema!(include_str!("parameters/desktop_snapshot.json"))
            }
            DesktopToolKind::Find => static_schema!(include_str!("parameters/desktop_find.json")),
            DesktopToolKind::Act => static_schema!(include_str!("parameters/desktop_act.json")),
            DesktopToolKind::Goal => static_schema!(include_str!("parameters/desktop_goal.json")),
            DesktopToolKind::ContinueGoal => {
                static_schema!(include_str!("parameters/desktop_continue_goal.json"))
            }
        }
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let thread_id = context
            .and_then(ToolRunContext::thread_id)
            .filter(|thread_id| !thread_id.is_empty());
        if !super::ops::listener_is_loopback() || !super::ops::enabled(&self.config) {
            return Ok(ToolResult::error(
                "Desktop control is disabled in Connections.",
            ));
        }
        let permission = crate::modules::desktop::permissions(&self.config).await;
        let permission = match permission {
            Ok(value) if value.ok => value,
            Ok(value) => {
                return Ok(ToolResult::error(format!(
                    "Desktop permission check failed: {}",
                    value
                        .error
                        .map_or_else(|| "unknown error".to_owned(), |error| error.message)
                )))
            }
            Err(error) => return Ok(ToolResult::error(error)),
        };
        let accessibility_raw = permission
            .data
            .as_ref()
            .and_then(|data| data.get("accessibility"))
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let accessibility =
            super::ops::normalize_permission_for_platform(accessibility_raw, std::env::consts::OS);
        if accessibility != "granted"
            && accessibility != "not_required"
            && !matches!(
                self.kind,
                DesktopToolKind::Apps | DesktopToolKind::Windows | DesktopToolKind::Launch
            )
        {
            return Ok(ToolResult::error("Desktop Accessibility permission is not granted. Enable it in system settings and retry."));
        }
        let mut goal_identity = None;
        let mut approvals_enabled = true;
        let (member, request) = match self.kind {
            DesktopToolKind::Apps => (
                names::methods::LIST_APPS,
                serde_json::to_value(ListAppsRequest::default())?,
            ),
            DesktopToolKind::Windows => (
                names::methods::LIST_WINDOWS,
                serde_json::to_value(ListWindowsRequest {
                    app: args.get("app").and_then(Value::as_str).map(str::to_owned),
                })?,
            ),
            DesktopToolKind::Launch => {
                let mut request = LaunchRequest::new(required(&args, "app")?);
                request.activate = true;
                request.attach_if_running = Some(true);
                (names::methods::LAUNCH, serde_json::to_value(request)?)
            }
            DesktopToolKind::Snapshot => {
                let request = snapshot_request(&args)?;
                (names::methods::SNAPSHOT, serde_json::to_value(request)?)
            }
            DesktopToolKind::Find => {
                let request = FindRequest {
                    app: args.get("app").and_then(Value::as_str).map(str::to_owned),
                    role: args.get("role").and_then(Value::as_str).map(str::to_owned),
                    name: args.get("name").and_then(Value::as_str).map(str::to_owned),
                    root: args.get("root").and_then(Value::as_str).map(str::to_owned),
                    limit: Some(
                        args.get("limit")
                            .and_then(Value::as_u64)
                            .unwrap_or(10)
                            .clamp(1, 20) as usize,
                    ),
                    ..FindRequest::default()
                };
                (names::methods::FIND, serde_json::to_value(request)?)
            }
            DesktopToolKind::Act => {
                let reference = required(&args, "ref_id")?;
                let operation = required(&args, "operation")?;
                if operation == "type" {
                    let text = required(&args, "text")?;
                    (
                        names::methods::TYPE,
                        serde_json::to_value(TypeRequest {
                            ref_id: reference,
                            text,
                            ..TypeRequest::default()
                        })?,
                    )
                } else {
                    let member = match operation.as_str() {
                        "click" => names::methods::CLICK,
                        "focus" => names::methods::FOCUS,
                        "check" => names::methods::CHECK,
                        "uncheck" => names::methods::UNCHECK,
                        "expand" => names::methods::EXPAND,
                        "collapse" => names::methods::COLLAPSE,
                        _ => return Ok(ToolResult::error("Unsupported desktop operation")),
                    };
                    (member, serde_json::to_value(RefRequest::new(reference))?)
                }
            }
            DesktopToolKind::Goal | DesktopToolKind::ContinueGoal => {
                let (app, goal, continuation) =
                    if matches!(self.kind, DesktopToolKind::ContinueGoal) {
                        let id = required(&args, "confirmation_id")?;
                        let (app, goal) = super::confirmation::take_approved(&id, thread_id)
                            .map_err(anyhow::Error::msg)?;
                        (app, goal, Some(json!({"id":id,"approve":true})))
                    } else {
                        (required(&args, "app")?, required(&args, "goal")?, None)
                    };
                goal_identity = Some((app.clone(), goal.clone()));
                let mut request = if continuation.is_some() {
                    json!({"app":app,"goal":goal})
                } else {
                    let live_config = match crate::config::rpc::load_config_with_timeout().await {
                        Ok(config) => config,
                        Err(error) => {
                            return Ok(ToolResult::error(format!(
                                "Desktop approval setting is unavailable: {error}"
                            )))
                        }
                    };
                    approvals_enabled = live_config.desktop.approvals_enabled;
                    goal_request(&args, approvals_enabled)?
                };
                if let Some(continuation) = continuation {
                    request["continuation"] = continuation;
                }
                (names::methods::RUN_GOAL, request)
            }
        };
        let reply = crate::modules::desktop::call(&self.config, member, request).await;
        match reply {
            Ok(response) if response.ok => {
                let data = response.data.unwrap_or(Value::Null);
                if matches!(self.kind, DesktopToolKind::Goal)
                    && !approvals_enabled
                    && data.get("stop").and_then(Value::as_str) == Some("confirmation_required")
                {
                    return Ok(ToolResult::error(
                        "Desktop module unexpectedly requested confirmation during an approvals-disabled task.",
                    ));
                }
                if let Some((app, goal)) = goal_identity.filter(|_| approvals_enabled) {
                    if data.get("stop").and_then(Value::as_str) == Some("confirmation_required") {
                        let Some(thread_id) = thread_id else {
                            return Ok(ToolResult::error(
                                "desktop confirmation requires a threaded agent run",
                            ));
                        };
                        super::confirmation::record(&app, &goal, thread_id, &data);
                    }
                }
                let rendered = serde_json::to_string(&data)?;
                // A bounded model result; full screenshots are not exposed through this tool.
                Ok(ToolResult::success(
                    rendered.chars().take(24_000).collect::<String>(),
                ))
            }
            Ok(response) => Ok(ToolResult::error(response.error.map_or_else(
                || "Desktop command failed without an error".to_owned(),
                |error| format!("{}: {}", error.code, error.message),
            ))),
            Err(error) => Ok(ToolResult::error(error)),
        }
    }
}

/// Build this domain's tools in their persisted catalogue order.
pub(crate) fn registered_tools(
    config: std::sync::Arc<crate::config::Config>,
) -> Vec<Box<dyn tinytools::Tool>> {
    vec![
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Apps)),
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Windows)),
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Launch)),
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Snapshot)),
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Find)),
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Goal)),
        Box::new(DesktopTool::new(
            config.clone(),
            DesktopToolKind::ContinueGoal,
        )),
    ]
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_schema_tests.rs"]
mod schema_tests;
