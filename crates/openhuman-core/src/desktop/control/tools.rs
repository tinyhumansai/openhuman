//! Deferred desktop tools. Only their names and descriptions enter discovery.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinycomputer_bus::desktop_tool::{self as declared, ActRequest, DesktopTool as Declared};
use tinycomputer_bus::{names, GoalContinuation, RunGoalRequest};
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolExposure, ToolResult, ToolRunContext, ToolTimeout,
};

use crate::config::Config;

/// The declared tools this host exposes. Names, descriptions, schemas and
/// argument decoding live in `tinycomputer_bus::desktop_tool`; this wrapper
/// adds the host's enablement, permission and approval policy.
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

impl DesktopToolKind {
    fn declared(self) -> Declared {
        match self {
            Self::Apps => Declared::Apps,
            Self::Windows => Declared::Windows,
            Self::Launch => Declared::Launch,
            Self::Snapshot => Declared::Snapshot,
            Self::Find => Declared::Find,
            Self::Act => Declared::Act,
            Self::Goal => Declared::Goal,
            Self::ContinueGoal => Declared::ContinueGoal,
        }
    }
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

fn to_value<T: serde::Serialize>(request: T) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(request)?)
}

#[async_trait]
impl Tool for DesktopTool {
    fn name(&self) -> &str {
        self.kind.declared().name()
    }

    fn description(&self) -> &str {
        self.kind.declared().description()
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn family(&self) -> Option<&str> {
        Some("desktop")
    }

    fn permission_level(&self) -> PermissionLevel {
        if self.kind.declared().mutates() {
            PermissionLevel::Write
        } else {
            PermissionLevel::ReadOnly
        }
    }

    fn external_effect(&self) -> bool {
        self.kind.declared().mutates()
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.kind
            .declared()
            .timeout_ms(args)
            .map_or(ToolTimeout::Inherit, ToolTimeout::Millis)
    }

    fn parameters_schema(&self) -> Value {
        self.kind.declared().parameters_schema()
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
        if !super::ops::platform_supported() {
            return Ok(ToolResult::error(
                "Desktop control is not available on this platform yet.",
            ));
        }
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
            && self.kind.declared().needs_accessibility()
        {
            return Ok(ToolResult::error("Desktop Accessibility permission is not granted. Enable it in system settings and retry."));
        }
        let mut goal_identity = None;
        let mut approvals_enabled = true;
        let built: Result<(&'static str, Value), anyhow::Error> = match self.kind {
            DesktopToolKind::Apps => {
                let (member, request) = declared::apps_request();
                to_value(request).map(|request| (member, request))
            }
            DesktopToolKind::Windows => {
                let (member, request) = declared::windows_request(&args);
                to_value(request).map(|request| (member, request))
            }
            DesktopToolKind::Launch => declared::launch_request(&args)
                .map_err(anyhow::Error::from)
                .and_then(|(member, request)| Ok((member, to_value(request)?))),
            DesktopToolKind::Snapshot => declared::snapshot_request(&args)
                .map_err(anyhow::Error::from)
                .and_then(|(member, request)| Ok((member, to_value(request)?))),
            DesktopToolKind::Find => {
                let (member, request) = declared::find_request(&args);
                to_value(request).map(|request| (member, request))
            }
            DesktopToolKind::Act => match declared::act_request(&args) {
                Ok((member, ActRequest::Type(request))) => {
                    to_value(request).map(|request| (member, request))
                }
                Ok((member, ActRequest::Ref(request))) => {
                    to_value(request).map(|request| (member, request))
                }
                Ok((member, _)) => Err(anyhow::anyhow!("unsupported act request for {member}")),
                Err(declared::ToolArgError::UnsupportedOperation) => {
                    return Ok(ToolResult::error("Unsupported desktop operation"))
                }
                Err(error) => Err(error.into()),
            },
            DesktopToolKind::Goal => {
                let live_config =
                    match crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(
                        context,
                    )
                    .load_config()
                    .await
                    {
                        Ok(config) => config,
                        Err(error) => {
                            return Ok(ToolResult::error(format!(
                                "Desktop approval setting is unavailable: {error}"
                            )))
                        }
                    };
                approvals_enabled = live_config.desktop.approvals_enabled;
                match declared::goal_request(&args, approvals_enabled) {
                    Ok((member, request)) => {
                        goal_identity = Some((request.app.clone(), request.goal.clone()));
                        to_value(request).map(|request| (member, request))
                    }
                    Err(error) => Err(error.into()),
                }
            }
            DesktopToolKind::ContinueGoal => {
                let id = declared::required(&args, "confirmation_id")?;
                let (app, goal) = super::confirmation::take_approved(&id, thread_id)
                    .map_err(anyhow::Error::msg)?;
                goal_identity = Some((app.clone(), goal.clone()));
                let request = RunGoalRequest {
                    app,
                    goal,
                    continuation: Some(GoalContinuation { id, approve: true }),
                    ..RunGoalRequest::default()
                };
                to_value(request).map(|request| (names::methods::RUN_GOAL, request))
            }
        };
        let (member, request) = built?;
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
