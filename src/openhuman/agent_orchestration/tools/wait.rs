//! Tool: `wait` / `wait_loop` - delayed callback ticks for the orchestrator.
//!
//! These tools intentionally do not own scheduling state. They sleep for a
//! bounded duration, then return the caller-provided message as a tool result so
//! the orchestrator can decide whether to act, poll async sub-agents, or call the
//! loop variant again.

use std::time::Duration;

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};
use async_trait::async_trait;
use serde_json::json;

const DEFAULT_DURATION_SECS: u64 = 5;
const MAX_DURATION_SECS: u64 = 600;
const MILLIS_PER_SEC: u64 = 1_000;

/// One-shot delayed callback tool for the orchestrator.
pub struct WaitTool;
/// Repeatable delayed callback tool for orchestrator-controlled polling loops.
pub struct WaitLoopTool;

impl WaitTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WaitTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitLoopTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WaitLoopTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WaitTool {
    fn name(&self) -> &str {
        "wait"
    }

    fn description(&self) -> &str {
        "Wait for a bounded duration, then return the provided callback message \
         to the orchestrator. Use this to create a delayed tick before checking \
         async work or retrying a condition."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        wait_schema(false)
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        timeout_policy_for_wait(args)
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        execute_wait(args, false).await
    }
}

#[async_trait]
impl Tool for WaitLoopTool {
    fn name(&self) -> &str {
        "wait_loop"
    }

    fn description(&self) -> &str {
        "Wait for a bounded duration, then return the same callback message plus \
         a ready-to-call wait_loop instruction. Use this for deliberate polling \
         loops where the orchestrator decides after each tick whether to repeat."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        wait_schema(true)
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn timeout_policy(&self, args: &serde_json::Value) -> ToolTimeout {
        timeout_policy_for_wait(args)
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        execute_wait(args, true).await
    }
}

/// Sleep for the requested duration, then return the callback tick payload.
async fn execute_wait(args: serde_json::Value, loop_mode: bool) -> anyhow::Result<ToolResult> {
    let request = match parse_wait_request(&args) {
        Ok(request) => request,
        Err(message) => return Ok(ToolResult::error(message)),
    };
    let tool_name = if loop_mode { "wait_loop" } else { "wait" };
    log::info!(
        "[wait] tool={} duration_ms={} iteration={} loop_key={} message_chars={}",
        tool_name,
        request.duration_ms,
        request.iteration,
        request.loop_key.as_deref().unwrap_or("none"),
        request.message.chars().count()
    );

    tokio::time::sleep(Duration::from_millis(request.duration_ms)).await;

    log::debug!(
        "[wait] elapsed tool={} duration_ms={} iteration={} loop_key={}",
        tool_name,
        request.duration_ms,
        request.iteration,
        request.loop_key.as_deref().unwrap_or("none")
    );

    Ok(ToolResult::success(format_wait_tick(&request, loop_mode)))
}

/// Build the JSON schema shared by `wait` and `wait_loop`.
fn wait_schema(loop_mode: bool) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "message".to_string(),
        json!({
            "type": "string",
            "description": "Callback message to return to the orchestrator after the wait elapses."
        }),
    );
    properties.insert(
        "duration_secs".to_string(),
        json!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_DURATION_SECS,
            "description": "Seconds to wait before returning the callback. Default 5. Ignored when duration_ms is supplied."
        }),
    );
    properties.insert(
        "duration_ms".to_string(),
        json!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_DURATION_SECS * MILLIS_PER_SEC,
            "description": "Milliseconds to wait before returning the callback. Use only for short test-sized waits."
        }),
    );

    if loop_mode {
        properties.insert(
            "loop_key".to_string(),
            json!({
                "type": "string",
                "description": "Optional caller-defined key for the polling loop."
            }),
        );
        properties.insert(
            "iteration".to_string(),
            json!({
                "type": "integer",
                "minimum": 1,
                "description": "Current loop iteration. Defaults to 1; the returned instruction increments it."
            }),
        );
    }

    json!({
        "type": "object",
        "required": ["message"],
        "properties": serde_json::Value::Object(properties)
    })
}

/// Give the harness a deadline that covers the requested wait plus grace.
fn timeout_policy_for_wait(args: &serde_json::Value) -> ToolTimeout {
    let duration_ms = duration_ms_from_args(args).unwrap_or(DEFAULT_DURATION_SECS * MILLIS_PER_SEC);
    let rounded_secs = duration_ms.saturating_add(MILLIS_PER_SEC - 1) / MILLIS_PER_SEC;
    ToolTimeout::Secs(rounded_secs.saturating_add(1))
}

/// Extract and clamp the caller's requested wait duration in milliseconds.
fn duration_ms_from_args(args: &serde_json::Value) -> Option<u64> {
    if let Some(duration_ms) = args.get("duration_ms").and_then(|v| v.as_u64()) {
        return Some(duration_ms.clamp(1, MAX_DURATION_SECS * MILLIS_PER_SEC));
    }
    args.get("duration_secs")
        .and_then(|v| v.as_u64())
        .map(|secs| secs.clamp(1, MAX_DURATION_SECS) * MILLIS_PER_SEC)
}

/// Validate tool arguments and normalize them into an internal request.
fn parse_wait_request(args: &serde_json::Value) -> Result<WaitRequest, String> {
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if message.is_empty() {
        return Err("wait: `message` is required".to_string());
    }

    let duration_ms = duration_ms_from_args(args).unwrap_or(DEFAULT_DURATION_SECS * MILLIS_PER_SEC);
    let iteration = args
        .get("iteration")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let loop_key = args
        .get("loop_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(WaitRequest {
        message,
        duration_ms,
        iteration,
        loop_key,
    })
}

/// Render the delayed callback as prose plus a machine-readable `[wait_tick]`.
fn format_wait_tick(request: &WaitRequest, loop_mode: bool) -> String {
    let seconds = request.duration_ms as f64 / MILLIS_PER_SEC as f64;
    let loop_instruction = loop_mode.then(|| {
        json!({
            "tool": "wait_loop",
            "arguments": {
                "message": request.message,
                "duration_ms": request.duration_ms,
                "loop_key": request.loop_key,
                "iteration": request.iteration + 1
            }
        })
    });
    let payload = json!({
        "status": "elapsed",
        "message": request.message,
        "duration_ms": request.duration_ms,
        "durationMs": request.duration_ms,
        "duration_secs": seconds,
        "durationSecs": seconds,
        "loop": loop_mode,
        "loop_key": request.loop_key,
        "loopKey": request.loop_key,
        "iteration": request.iteration,
        "instructions": {
            "callback_message": request.message,
            "repeat": loop_instruction
        }
    });

    let prefix = if loop_mode {
        format!(
            "Loop tick {} elapsed after {}ms.",
            request.iteration, request.duration_ms
        )
    } else {
        format!("Wait elapsed after {}ms.", request.duration_ms)
    };

    format!(
        "{prefix}\n\n[wait_tick]\n{}\n[/wait_tick]\n\n{}",
        serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
        request.message
    )
}

#[derive(Debug)]
/// Normalized wait arguments used by both wait tools.
struct WaitRequest {
    message: String,
    duration_ms: u64,
    iteration: u64,
    loop_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_schema_requires_message() {
        let schema = WaitTool::new().parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required list");
        assert!(required.iter().any(|v| v.as_str() == Some("message")));
        assert!(schema["properties"].get("duration_secs").is_some());
        assert!(schema["properties"].get("duration_ms").is_some());
    }

    #[test]
    fn wait_loop_schema_includes_loop_controls() {
        let schema = WaitLoopTool::new().parameters_schema();
        assert!(schema["properties"].get("loop_key").is_some());
        assert!(schema["properties"].get("iteration").is_some());
    }

    #[test]
    fn parse_wait_request_clamps_duration_and_iteration() {
        let request = parse_wait_request(&json!({
            "message": "check subagents",
            "duration_secs": 9999,
            "iteration": 0
        }))
        .unwrap();
        assert_eq!(request.duration_ms, MAX_DURATION_SECS * MILLIS_PER_SEC);
        assert_eq!(request.iteration, 1);
    }

    #[test]
    fn missing_message_is_rejected() {
        let err = parse_wait_request(&json!({ "duration_secs": 1 })).unwrap_err();
        assert!(err.contains("message"));
    }

    #[test]
    fn wait_loop_tick_repeats_same_message() {
        let request = parse_wait_request(&json!({
            "message": "poll async workers",
            "duration_ms": 10,
            "loop_key": "workers",
            "iteration": 2
        }))
        .unwrap();
        let output = format_wait_tick(&request, true);

        assert!(output.contains("Loop tick 2 elapsed"));
        assert!(output.contains("\"tool\":\"wait_loop\""));
        assert!(output.contains("\"message\":\"poll async workers\""));
        assert!(output.contains("\"iteration\":3"));
    }

    #[tokio::test]
    async fn wait_execute_returns_callback_message() {
        let res = WaitTool::new()
            .execute(json!({
                "message": "time to check",
                "duration_ms": 1
            }))
            .await
            .unwrap();
        assert!(!res.is_error);
        assert!(res.output().contains("[wait_tick]"));
        assert!(res.output().contains("time to check"));
    }
}
