//! Golden request bodies for the prompt-cache layout.
//!
//! Drives the real tinyagents loop (with the host's `run_policy_for` policy and
//! the crate `PromptCacheGuardMiddleware`) against a recording mock model and
//! compares the exact `ModelRequest` each provider call receives (messages,
//! tools, `cache_segments`, `prompt_fingerprint`, `provider_options`
//! including `prompt_cache_key`) with `fixtures/prompt_cache_golden.json`.
//! Provider adapters are pure functions of that request, so identical requests
//! render byte-identical `cache_control` placement.
//!
//! The fixture was captured with the former host
//! `PromptCacheSegmentMiddleware` installed (and the host-side frozen-prefix
//! field set); it was then deleted because the vendor loop already owns the
//! frozen prefix, and the fixture must not change. Regenerate (only on a deliberate, reviewed
//! contract change) with `UPDATE_PROMPT_CACHE_GOLDEN=1`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents_harness::context::RunConfig;
use tinyagents_harness::middleware::PromptCacheGuardMiddleware;
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::testkit::ScriptedModel;
use tinyinference_llm::message::{AssistantMessage, Message};
use tinyinference_llm::model::ModelResponse;
use tinyinference_llm::tool::ToolCall;
use tinytools::{Tool, ToolResult};

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::turn_policy::run_policy_for;

struct Lookup;

#[async_trait]
impl Tool for Lookup {
    fn name(&self) -> &str {
        "lookup"
    }
    fn description(&self) -> &str {
        "Looks up a record."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}})
    }
    async fn execute(&self, _arguments: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("found"))
    }
}

fn tool_call_reply() -> ModelResponse {
    let mut response = ModelResponse::assistant("");
    response.message = AssistantMessage {
        id: Some("a1".into()),
        content: Vec::new(),
        tool_calls: vec![ToolCall::new("c1", "lookup", json!({"id":"7"}))],
        usage: None,
        origin: None,
    };
    response.finish_reason = Some("tool_calls".into());
    response
}

struct Scenario {
    name: &'static str,
    input: Vec<Message>,
    /// Frozen system-prefix length a resumed session carries.
    frozen: Option<usize>,
    with_tools: bool,
    /// First model call asks for the tool, so the loop makes two calls.
    tool_loop: bool,
    dialect: tinyagents_harness::config::ToolDispatcher,
}

async fn run(s: &Scenario) -> Value {
    let model = Arc::new(ScriptedModel::new(if s.tool_loop {
        vec![tool_call_reply(), ModelResponse::assistant("done")]
    } else {
        vec![ModelResponse::assistant("done")]
    }));
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    harness.with_model(model.clone());
    if s.with_tools {
        harness.register_tool(Arc::new(Lookup));
    }
    let mut policy = run_policy_for(8, false);
    policy.tool_dialect = s.dialect;
    harness.with_policy(policy);
    harness.push_middleware(Arc::new(PromptCacheGuardMiddleware::new()));

    let host = OpenHumanRunContext::new().with_tool_dialect(s.dialect);
    let mut ctx = host.into_tinyagents(RunConfig::new("golden"));
    if let Some(frozen) = s.frozen {
        ctx = ctx.with_frozen_system_prefix_len(frozen);
    }
    harness
        .invoke_in_context(&(), ctx, s.input.clone())
        .await
        .expect("scenario run succeeds");
    json!({
        "scenario": s.name,
        "requests": model.requests().iter().map(|r| serde_json::to_value(r).unwrap()).collect::<Vec<_>>(),
    })
}

fn scenarios() -> Vec<Scenario> {
    use tinyagents_harness::config::ToolDispatcher::{Auto, Python};
    let sys2 = || {
        vec![
            Message::system("stable+context"),
            Message::system("volatile tier"),
        ]
    };
    vec![
        Scenario {
            name: "first_turn_with_tools",
            input: sys2().into_iter().chain([Message::user("hi")]).collect(),
            frozen: None,
            with_tools: true,
            tool_loop: true,
            dialect: Auto,
        },
        Scenario {
            name: "first_turn_no_tools",
            input: vec![Message::system("only"), Message::user("hi")],
            frozen: None,
            with_tools: false,
            tool_loop: false,
            dialect: Auto,
        },
        Scenario {
            name: "first_turn_text_dialect",
            input: sys2().into_iter().chain([Message::user("hi")]).collect(),
            frozen: None,
            with_tools: true,
            tool_loop: false,
            dialect: Python,
        },
        Scenario {
            name: "resumed_session",
            input: sys2()
                .into_iter()
                .chain([
                    Message::user("hi"),
                    Message::assistant("hello"),
                    Message::user("and again"),
                ])
                .collect(),
            frozen: Some(2),
            with_tools: true,
            tool_loop: true,
            dialect: Auto,
        },
        Scenario {
            name: "after_compaction_summary",
            input: sys2()
                .into_iter()
                .chain([
                    Message::system("compaction summary of earlier turns"),
                    Message::user("continue"),
                ])
                .collect(),
            frozen: Some(2),
            with_tools: true,
            tool_loop: true,
            dialect: Auto,
        },
        Scenario {
            name: "resumed_unrecoverable_prefix",
            input: vec![Message::system("history summary"), Message::user("go")],
            frozen: Some(0),
            with_tools: true,
            tool_loop: false,
            dialect: Auto,
        },
        Scenario {
            name: "resumed_zero_prefix_no_system_messages",
            input: vec![Message::user("go")],
            frozen: Some(0),
            with_tools: false,
            tool_loop: false,
            dialect: Auto,
        },
        Scenario {
            name: "subagent_fresh_context",
            input: vec![Message::system("sub-agent prompt"), Message::user("task")],
            frozen: None,
            with_tools: true,
            tool_loop: true,
            dialect: Auto,
        },
        Scenario {
            name: "fresh_run_with_system_nudge_after_user",
            input: vec![
                Message::system("stable"),
                Message::user("work"),
                Message::system("[no progress] change strategy"),
            ],
            frozen: None,
            with_tools: true,
            tool_loop: false,
            dialect: Auto,
        },
    ]
}

#[tokio::test]
async fn prompt_cache_request_bodies_match_golden() {
    let mut out = Vec::new();
    for scenario in scenarios() {
        out.push(run(&scenario).await);
    }
    let actual = serde_json::to_string_pretty(&Value::Array(out)).unwrap() + "\n";
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/agent/tinyagents/fixtures/prompt_cache_golden.json"
    );
    if std::env::var_os("UPDATE_PROMPT_CACHE_GOLDEN").is_some() {
        std::fs::write(path, &actual).unwrap();
        return;
    }
    assert_eq!(
        actual,
        include_str!("fixtures/prompt_cache_golden.json"),
        "prompt-cache request layout changed; see module docs"
    );
}
