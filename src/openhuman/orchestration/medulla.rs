//! Client for the backend's paid Medulla request/response surface.
//!
//! OpenHuman supplies its existing local orchestration tools to
//! `/orchestration/v1/run`, executes requested calls on-device, and returns the
//! results through `/run/continue` until the cycle ends.

use std::sync::Arc;

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::api::config::effective_backend_api_url;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::{Config, MedullaClientConfig};
use crate::openhuman::tools::Tool;

const LOG: &str = "orchestration";
const RUN_PATH: &str = "/orchestration/v1/run";
const CONTINUE_PATH: &str = "/orchestration/v1/run/continue";
const PLAN_PATH: &str = "/payments/stripe/currentPlan";
const MAX_LOOP_EVENTS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MedullaRunResult {
    pub reply: String,
    #[serde(default)]
    pub pass_count: u32,
    #[serde(default)]
    pub compressed_history: Vec<String>,
    #[serde(default)]
    pub escalations: Vec<String>,
    pub session_id: String,
    pub cycle_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCall {
    id: String,
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "stop", rename_all = "snake_case")]
enum LoopEvent {
    ToolUse {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolCalls")]
        tool_calls: Vec<ToolCall>,
    },
    End {
        #[serde(flatten)]
        result: MedullaRunResult,
    },
    Pending {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Error {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        error: String,
    },
}

/// Run a Medulla cycle using the signed-in user's paid backend plan. The local
/// plan lookup is a fail-fast UX guard; the backend remains authoritative and
/// repeats the paid-plan check on both run endpoints.
pub async fn run(
    config: &Config,
    input: &str,
    session_id: Option<&str>,
) -> Result<MedullaRunResult, String> {
    if !config.orchestration.enabled {
        return Err("hosted orchestration is disabled in config".to_string());
    }
    let input = input.trim();
    if input.is_empty() {
        return Err("input is required".to_string());
    }

    let token = crate::openhuman::credentials::session_support::require_live_session_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|err| err.to_string())?;
    ensure_paid_plan(&client, &token).await?;

    let tuning = config.orchestration.medulla.clone();
    let config = Arc::new(config.clone());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(super::tools::ListContactsTool),
        Box::new(super::tools::ListSessionsTool::new(Arc::clone(&config))),
        Box::new(super::tools::ReadSessionTool::new(Arc::clone(&config))),
        Box::new(super::tools::SendToAgentTool::new(config)),
    ];

    run_with_client(&client, &token, input, session_id, &tools, &tuning).await
}

async fn ensure_paid_plan(client: &BackendOAuthClient, token: &str) -> Result<(), String> {
    let data = client
        .authed_json(token, Method::GET, PLAN_PATH, None)
        .await
        .map_err(crate::api::flatten_authed_error)?;
    if paid_plan_active(&data) {
        return Ok(());
    }
    Err("Medulla orchestration requires an active Basic or Pro plan".to_string())
}

fn paid_plan_active(data: &Value) -> bool {
    let plan = data.get("plan").and_then(Value::as_str).unwrap_or_default();
    let active = data
        .get("hasActiveSubscription")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    active && matches!(plan.to_ascii_uppercase().as_str(), "BASIC" | "PRO")
}

async fn run_with_client(
    client: &BackendOAuthClient,
    token: &str,
    input: &str,
    session_id: Option<&str>,
    tools: &[Box<dyn Tool>],
    tuning: &MedullaClientConfig,
) -> Result<MedullaRunResult, String> {
    let mut body = Map::new();
    body.insert("input".to_string(), Value::String(input.to_string()));
    if let Some(session_id) = session_id.map(str::trim).filter(|id| !id.is_empty()) {
        body.insert(
            "sessionId".to_string(),
            Value::String(session_id.to_string()),
        );
    }
    if !tools.is_empty() {
        body.insert(
            "tools".to_string(),
            Value::Array(tools.iter().map(|tool| tool_spec(tool.as_ref())).collect()),
        );
    }
    if let Some(options) = tuning_value(tuning) {
        body.insert("options".to_string(), options);
    }

    tracing::debug!(
        input_bytes = input.len(),
        tool_count = tools.len(),
        has_session = session_id.is_some(),
        "[orchestration] medulla.run.start"
    );
    let first = post(client, token, RUN_PATH, Value::Object(body)).await?;
    if tools.is_empty() {
        let result = serde_json::from_value(first)
            .map_err(|err| format!("parse Medulla run response: {err}"))?;
        tracing::debug!("[orchestration] medulla.run.end direct=true");
        return Ok(result);
    }

    drive_tool_loop(client, token, tools, first).await
}

async fn drive_tool_loop(
    client: &BackendOAuthClient,
    token: &str,
    tools: &[Box<dyn Tool>],
    mut value: Value,
) -> Result<MedullaRunResult, String> {
    for event_index in 0..MAX_LOOP_EVENTS {
        let event: LoopEvent = serde_json::from_value(value)
            .map_err(|err| format!("parse Medulla loop event: {err}"))?;
        match event {
            LoopEvent::End { result } => {
                tracing::debug!(
                    cycle_id = %result.cycle_id,
                    pass_count = result.pass_count,
                    events = event_index + 1,
                    "[orchestration] medulla.run.end"
                );
                return Ok(result);
            }
            LoopEvent::Error { cycle_id, error } => {
                tracing::warn!(cycle_id = %cycle_id, "[orchestration] medulla.run.error");
                return Err(format!("Medulla cycle {cycle_id} failed: {error}"));
            }
            LoopEvent::Pending {
                cycle_id,
                session_id,
            } => {
                tracing::debug!(
                    cycle_id = %cycle_id,
                    session_id = %session_id,
                    "[orchestration] medulla.run.pending"
                );
                value = continue_run(client, token, &cycle_id, Vec::new()).await?;
            }
            LoopEvent::ToolUse {
                cycle_id,
                session_id,
                tool_calls,
            } => {
                tracing::debug!(
                    cycle_id = %cycle_id,
                    session_id = %session_id,
                    call_count = tool_calls.len(),
                    "[orchestration] medulla.run.tool_use"
                );
                let mut results = Vec::with_capacity(tool_calls.len());
                for call in tool_calls {
                    results.push(execute_tool_call(tools, call).await);
                }
                value = continue_run(client, token, &cycle_id, results).await?;
            }
        }
    }
    Err(format!(
        "Medulla tool loop exceeded {MAX_LOOP_EVENTS} events"
    ))
}

async fn execute_tool_call(tools: &[Box<dyn Tool>], call: ToolCall) -> Value {
    let Some(tool) = tools.iter().find(|tool| tool.name() == call.name) else {
        tracing::warn!(tool = %call.name, "[orchestration] medulla.tool.unknown");
        return json!({
            "id": call.id,
            "ok": false,
            "error": format!("unknown OpenHuman tool: {}", call.name),
        });
    };

    tracing::debug!(tool = %call.name, call_id = %call.id, "[orchestration] medulla.tool.start");
    match tool.execute(call.args).await {
        Ok(result) if !result.is_error => {
            tracing::debug!(tool = %call.name, call_id = %call.id, "[orchestration] medulla.tool.end");
            json!({ "id": call.id, "ok": true, "result": result.output_for_llm(true) })
        }
        Ok(result) => {
            tracing::warn!(tool = %call.name, call_id = %call.id, "[orchestration] medulla.tool.failed");
            json!({ "id": call.id, "ok": false, "error": result.output() })
        }
        Err(err) => {
            tracing::warn!(tool = %call.name, call_id = %call.id, error = %err, "[orchestration] medulla.tool.failed");
            json!({ "id": call.id, "ok": false, "error": err.to_string() })
        }
    }
}

async fn continue_run(
    client: &BackendOAuthClient,
    token: &str,
    cycle_id: &str,
    tool_results: Vec<Value>,
) -> Result<Value, String> {
    post(
        client,
        token,
        CONTINUE_PATH,
        json!({ "cycleId": cycle_id, "toolResults": tool_results }),
    )
    .await
}

async fn post(
    client: &BackendOAuthClient,
    token: &str,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    client
        .authed_json(token, Method::POST, path, Some(body))
        .await
        .map_err(crate::api::flatten_authed_error)
}

fn tool_spec(tool: &dyn Tool) -> Value {
    json!({
        "name": tool.name(),
        "description": tool.description(),
        "parameters": tool.parameters_schema(),
    })
}

fn tuning_value(tuning: &MedullaClientConfig) -> Option<Value> {
    let prompt_overrides = &tuning.prompt_overrides;
    let prompt_overrides = json_object([
        (
            "ORCHESTRATE_SYSTEM",
            prompt_overrides
                .orchestrate_system
                .as_ref()
                .map(|v| json!(v)),
        ),
        (
            "REASONING_EXECUTE_SYSTEM",
            prompt_overrides
                .reasoning_execute_system
                .as_ref()
                .map(|v| json!(v)),
        ),
        (
            "ORCHESTRATE_RLM_SYSTEM",
            prompt_overrides
                .orchestrate_rlm_system
                .as_ref()
                .map(|v| json!(v)),
        ),
        (
            "COMPRESS_SYSTEM",
            prompt_overrides.compress_system.as_ref().map(|v| json!(v)),
        ),
        (
            "FRONTEND_GATE_SYSTEM",
            prompt_overrides
                .frontend_gate_system
                .as_ref()
                .map(|v| json!(v)),
        ),
    ]);
    let config = &tuning.config;
    let config = json_object([
        ("maxPasses", config.max_passes.map(|v| json!(v))),
        ("maxSteps", config.max_steps.map(|v| json!(v))),
        ("maxDepth", config.max_depth.map(|v| json!(v))),
        (
            "contextWindowTokens",
            config.context_window_tokens.map(|v| json!(v)),
        ),
        (
            "verification",
            config.verification.map(|v| {
                json!(match v {
                    crate::openhuman::config::MedullaVerification::Remind => "remind",
                    crate::openhuman::config::MedullaVerification::Off => "off",
                })
            }),
        ),
    ]);
    let limits = &tuning.limits;
    let limits = json_object([
        ("maxConcurrency", limits.max_concurrency.map(|v| json!(v))),
        ("maxTokens", limits.max_tokens.map(|v| json!(v))),
        ("deadlineMs", limits.deadline_ms.map(|v| json!(v))),
        (
            "maxTasksPerDelegate",
            limits.max_tasks_per_delegate.map(|v| json!(v)),
        ),
        ("maxDepth", limits.max_depth.map(|v| json!(v))),
    ]);
    let options = json_object([
        ("promptOverrides", prompt_overrides.map(Value::Object)),
        ("config", config.map(Value::Object)),
        ("limits", limits.map(Value::Object)),
    ]);
    options.map(Value::Object)
}

fn json_object<const N: usize>(entries: [(&str, Option<Value>); N]) -> Option<Map<String, Value>> {
    let object: Map<String, Value> = entries
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key.to_string(), value)))
        .collect();
    (!object.is_empty()).then_some(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::openhuman::config::{MedullaCycleConfig, MedullaPromptOverrides};
    use crate::openhuman::tools::ToolResult;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo text"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})
        }

        async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success(
                args.get("text").and_then(Value::as_str).unwrap_or_default(),
            ))
        }
    }

    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn name(&self) -> &str {
            "fail"
        }

        fn description(&self) -> &str {
            "Return a tool-level failure"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type":"object"})
        }

        async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::error("expected failure"))
        }
    }

    struct ExplodingTool;

    #[async_trait]
    impl Tool for ExplodingTool {
        fn name(&self) -> &str {
            "explode"
        }

        fn description(&self) -> &str {
            "Return an execution error"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type":"object"})
        }

        async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
            anyhow::bail!("execution exploded")
        }
    }

    fn end_event() -> Value {
        json!({
            "stop":"end",
            "reply":"done",
            "passCount":2,
            "compressedHistory":[],
            "escalations":[],
            "cycleId":"cycle-1",
            "sessionId":"session-1"
        })
    }

    fn envelope(data: Value) -> Value {
        json!({"success": true, "data": data})
    }

    #[test]
    fn paid_plan_requires_active_basic_or_pro() {
        assert!(paid_plan_active(
            &json!({"plan":"PRO","hasActiveSubscription":true})
        ));
        assert!(paid_plan_active(
            &json!({"plan":"basic","hasActiveSubscription":true})
        ));
        assert!(!paid_plan_active(
            &json!({"plan":"FREE","hasActiveSubscription":true})
        ));
        assert!(!paid_plan_active(
            &json!({"plan":"PRO","hasActiveSubscription":false})
        ));
    }

    #[test]
    fn tuning_uses_backend_field_names_and_omits_empty_sections() {
        assert!(tuning_value(&MedullaClientConfig::default()).is_none());
        let tuning = MedullaClientConfig {
            prompt_overrides: MedullaPromptOverrides {
                orchestrate_system: Some("custom".to_string()),
                ..Default::default()
            },
            config: MedullaCycleConfig {
                max_passes: Some(3),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            tuning_value(&tuning),
            Some(json!({
                "promptOverrides": {"ORCHESTRATE_SYSTEM":"custom"},
                "config": {"maxPasses":3}
            }))
        );
    }

    #[tokio::test]
    async fn paid_plan_check_accepts_paid_and_rejects_free() {
        for (plan, active, should_pass) in [("PRO", true, true), ("FREE", true, false)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(PLAN_PATH))
                .and(header("authorization", "Bearer test-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
                    "plan": plan,
                    "hasActiveSubscription": active
                }))))
                .mount(&server)
                .await;

            let client = BackendOAuthClient::new(&server.uri()).unwrap();
            let result = ensure_paid_plan(&client, "test-token").await;
            assert_eq!(result.is_ok(), should_pass);
        }
    }

    #[tokio::test]
    async fn public_run_validates_enabled_and_input_before_credentials() {
        let mut config = Config::default();
        config.orchestration.enabled = false;
        assert_eq!(
            run(&config, "task", None).await.unwrap_err(),
            "hosted orchestration is disabled in config"
        );

        config.orchestration.enabled = true;
        assert_eq!(
            run(&config, "  ", None).await.unwrap_err(),
            "input is required"
        );
    }

    #[tokio::test]
    async fn run_without_tools_returns_direct_result_and_forwards_options() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(RUN_PATH))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(end_event())))
            .mount(&server)
            .await;

        let client = BackendOAuthClient::new(&server.uri()).unwrap();
        let tuning = MedullaClientConfig {
            config: MedullaCycleConfig {
                max_passes: Some(4),
                ..Default::default()
            },
            ..Default::default()
        };
        let result = run_with_client(
            &client,
            "test-token",
            "direct",
            Some(" session-1 "),
            &[],
            &tuning,
        )
        .await
        .unwrap();

        assert_eq!(result.reply, "done");
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["sessionId"], "session-1");
        assert_eq!(body["options"]["config"]["maxPasses"], 4);
        assert!(body.get("tools").is_none());
    }

    #[tokio::test]
    async fn pending_event_polls_until_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(CONTINUE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(end_event())))
            .mount(&server)
            .await;
        let client = BackendOAuthClient::new(&server.uri()).unwrap();

        let result = drive_tool_loop(
            &client,
            "test-token",
            &[Box::new(EchoTool)],
            json!({"stop":"pending","cycleId":"cycle-1","sessionId":"session-1"}),
        )
        .await
        .unwrap();

        assert_eq!(result.reply, "done");
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body, json!({"cycleId":"cycle-1","toolResults":[]}));
    }

    #[tokio::test]
    async fn error_event_is_returned_with_cycle_context() {
        let server = MockServer::start().await;
        let client = BackendOAuthClient::new(&server.uri()).unwrap();
        let error = drive_tool_loop(
            &client,
            "test-token",
            &[],
            json!({"stop":"error","cycleId":"cycle-bad","error":"budget exceeded"}),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Medulla cycle cycle-bad failed: budget exceeded");
    }

    #[tokio::test]
    async fn unknown_and_failed_tools_are_reported_to_backend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(CONTINUE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(end_event())))
            .mount(&server)
            .await;
        let client = BackendOAuthClient::new(&server.uri()).unwrap();
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(FailingTool), Box::new(ExplodingTool)];

        drive_tool_loop(
            &client,
            "test-token",
            &tools,
            json!({
                "stop":"tool_use",
                "cycleId":"cycle-1",
                "sessionId":"session-1",
                "toolCalls":[
                    {"id":"call-unknown","name":"missing","args":{}},
                    {"id":"call-failed","name":"fail","args":{}},
                    {"id":"call-exploded","name":"explode","args":{}}
                ]
            }),
        )
        .await
        .unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["toolResults"][0]["ok"], false);
        assert_eq!(
            body["toolResults"][0]["error"],
            "unknown OpenHuman tool: missing"
        );
        assert_eq!(body["toolResults"][1]["error"], "expected failure");
        assert_eq!(body["toolResults"][2]["error"], "execution exploded");
    }

    #[tokio::test]
    async fn tool_loop_executes_locally_and_continues_to_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(RUN_PATH))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": {
                    "stop":"tool_use",
                    "cycleId":"cycle-1",
                    "sessionId":"session-1",
                    "toolCalls":[{"id":"call-1","name":"echo","args":{"text":"hello"}}]
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(CONTINUE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(envelope(end_event())))
            .mount(&server)
            .await;

        let client = BackendOAuthClient::new(&server.uri()).unwrap();
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let result = run_with_client(
            &client,
            "test-token",
            "use echo",
            None,
            &tools,
            &MedullaClientConfig::default(),
        )
        .await
        .unwrap();

        assert_eq!(result.reply, "done");
        assert_eq!(result.pass_count, 2);
        let requests = server.received_requests().await.unwrap();
        let continued: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(continued["toolResults"][0]["result"], "hello");
    }
}
