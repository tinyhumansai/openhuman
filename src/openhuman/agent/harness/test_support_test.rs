//! Behavioural tests of the agent harness driven by the smart mock
//! provider in [`super::test_support`]. These exercise the real
//! [`run_tool_call_loop`] path end-to-end — no provider stubbing inside
//! the test bodies — and surface regressions in tool dispatch, parsing,
//! and history threading.

use super::test_support::{
    spawn_fake_composio_backend, ComposioExecuteRule, ComposioFixture, KeywordRule,
    KeywordScriptedProvider, ScriptedToolCall,
};
use super::tool_loop::run_tool_call_loop;
use crate::openhuman::inference::provider::{ChatMessage, ChatRequest, ChatResponse, Provider};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn mm() -> crate::openhuman::config::MultimodalConfig {
    crate::openhuman::config::MultimodalConfig::default()
}

#[tokio::test]
async fn keyword_provider_records_forced_then_fallback_turns() {
    let provider =
        KeywordScriptedProvider::new(vec![KeywordRule::final_reply("matched", "final answer")])
            .with_native_tools(true)
            .with_vision(true)
            .with_fallback("fallback reply");

    let caps = provider.capabilities();
    assert!(caps.native_tool_calling);
    assert!(caps.vision);

    provider.push_forced_response(ChatResponse {
        text: Some("forced reply".into()),
        tool_calls: vec![],
        usage: None,
    });

    let messages = vec![ChatMessage::user("nothing should match here")];
    let forced = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: None,
            },
            "test-model",
            0.0,
        )
        .await
        .expect("forced response");
    assert_eq!(forced.text.as_deref(), Some("forced reply"));

    let fallback = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: None,
            },
            "test-model",
            0.0,
        )
        .await
        .expect("fallback response");
    assert_eq!(fallback.text.as_deref(), Some("fallback reply"));

    let turns = provider.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].rule_keyword, None);
    assert_eq!(turns[0].emitted_text.as_deref(), Some("forced reply"));
    assert_eq!(turns[1].rule_keyword, None);
    assert_eq!(turns[1].emitted_text.as_deref(), Some("fallback reply"));
}

#[tokio::test]
async fn keyword_provider_prompt_guided_text_wraps_tool_calls_and_honors_fire_limit() {
    let provider = KeywordScriptedProvider::new(vec![KeywordRule::tool_call(
        "search please",
        ScriptedToolCall::new("search_tool", json!({"q": "rust"})),
    )
    .with_text("Looking it up.")]);

    let messages = vec![
        ChatMessage::assistant("earlier assistant turn"),
        ChatMessage::tool("search please from a tool result"),
    ];

    let first = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: None,
            },
            "test-model",
            0.0,
        )
        .await
        .expect("prompt-guided response");

    let text = first.text.expect("prompt-guided text body");
    assert!(first.tool_calls.is_empty());
    assert!(text.starts_with("Looking it up.\n"));
    assert!(text.contains("<tool_call>"));
    assert!(text.contains("\"name\":\"search_tool\""));
    assert!(text.contains("\"q\":\"rust\""));

    let second = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: None,
            },
            "test-model",
            0.0,
        )
        .await
        .expect("fallback after max_fires");
    assert_eq!(second.text.as_deref(), Some("done"));
    assert_eq!(provider.turn_count(), 2);
}

#[tokio::test]
async fn fake_composio_backend_serves_routes_and_uses_response_fallbacks() {
    let mut fixture = ComposioFixture::realistic();
    fixture.execute_rules = vec![ComposioExecuteRule::new(
        "GMAIL_FETCH_EMAILS",
        json!({"messages": [{"id": "gmail-priority-2"}]}),
    )
    .when_argument_contains("arguments.query", "release blocker")];

    let backend = spawn_fake_composio_backend(fixture).await;
    let http = reqwest::Client::new();

    let toolkits: serde_json::Value = http
        .get(format!(
            "{}/agent-integrations/composio/toolkits",
            backend.base_url
        ))
        .send()
        .await
        .expect("toolkits request")
        .json()
        .await
        .expect("toolkits json");
    assert_eq!(toolkits["data"]["toolkits"][0], "gmail");

    let authorize: serde_json::Value = http
        .post(format!(
            "{}/agent-integrations/composio/authorize",
            backend.base_url
        ))
        .json(&json!({"toolkit": "gmail"}))
        .send()
        .await
        .expect("authorize request")
        .json()
        .await
        .expect("authorize json");
    assert_eq!(authorize["data"]["connectionId"], "conn_gmail_pending",);

    let rule_match: serde_json::Value = http
        .post(format!(
            "{}/agent-integrations/composio/execute",
            backend.base_url
        ))
        .json(&json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "arguments": {"query": "Need RELEASE BLOCKER updates"},
        }))
        .send()
        .await
        .expect("execute request")
        .json()
        .await
        .expect("execute json");
    assert_eq!(
        rule_match["data"]["data"]["messages"][0]["id"],
        "gmail-priority-2",
    );

    let execute_fallback: serde_json::Value = http
        .post(format!(
            "{}/agent-integrations/composio/execute",
            backend.base_url
        ))
        .json(&json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "arguments": {"page": 1},
        }))
        .send()
        .await
        .expect("execute fallback request")
        .json()
        .await
        .expect("execute fallback json");
    assert_eq!(execute_fallback["data"]["data"]["messages"][0]["id"], "m1",);

    let default_execute: serde_json::Value = http
        .post(format!(
            "{}/agent-integrations/composio/execute",
            backend.base_url
        ))
        .json(&json!({
            "tool": "UNKNOWN_ACTION",
            "arguments": {"topic": "ops"},
        }))
        .send()
        .await
        .expect("default execute request")
        .json()
        .await
        .expect("default execute json");
    assert_eq!(default_execute["data"]["data"]["ok"], true);
    assert_eq!(default_execute["data"]["data"]["action"], "UNKNOWN_ACTION");

    let delete: serde_json::Value = http
        .delete(format!(
            "{}/agent-integrations/composio/connections/conn_gmail_1",
            backend.base_url
        ))
        .send()
        .await
        .expect("delete request")
        .json()
        .await
        .expect("delete json");
    assert_eq!(delete["data"]["deleted"], true);

    let requests = backend.requests();
    assert!(
        requests
            .iter()
            .any(|(m, p, _)| m == "GET" && p == "/toolkits"),
        "expected toolkits route to be recorded"
    );
    assert!(
        requests
            .iter()
            .any(|(m, p, _)| m == "POST" && p == "/authorize"),
        "expected authorize route to be recorded"
    );
    assert!(
        requests
            .iter()
            .any(|(m, p, body)| m == "POST" && p == "/execute" && body["tool"] == "UNKNOWN_ACTION"),
        "expected execute route to record unknown action body"
    );
    assert!(
        requests
            .iter()
            .any(|(m, p, _)| m == "DELETE" && p == "/connections/conn_gmail_1"),
        "expected delete route to be recorded"
    );
}

/// Generic test tool: records the args it was called with and returns
/// whatever was wired at construction.
struct RecordingTool {
    name_str: String,
    description_str: String,
    result: ToolResult,
    calls: Arc<parking_lot::Mutex<Vec<serde_json::Value>>>,
    permission: PermissionLevel,
    scope_v: ToolScope,
    category_v: ToolCategory,
}

impl RecordingTool {
    fn echo(name: &str) -> (Self, Arc<parking_lot::Mutex<Vec<serde_json::Value>>>) {
        let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let tool = Self {
            name_str: name.to_string(),
            description_str: format!("recording tool {name}"),
            result: ToolResult::success(format!("{name}-ok")),
            calls: calls.clone(),
            permission: PermissionLevel::ReadOnly,
            scope_v: ToolScope::All,
            category_v: ToolCategory::System,
        };
        (tool, calls)
    }
}

struct SequencedTool {
    name_str: String,
    result: ToolResult,
    calls: Arc<parking_lot::Mutex<Vec<serde_json::Value>>>,
    sequence: Arc<parking_lot::Mutex<Vec<String>>>,
}

impl SequencedTool {
    fn new(
        name: &str,
        result: ToolResult,
        sequence: Arc<parking_lot::Mutex<Vec<String>>>,
    ) -> (Self, Arc<parking_lot::Mutex<Vec<serde_json::Value>>>) {
        let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        (
            Self {
                name_str: name.to_string(),
                result,
                calls: calls.clone(),
                sequence,
            },
            calls,
        )
    }
}

#[async_trait]
impl Tool for SequencedTool {
    fn name(&self) -> &str {
        &self.name_str
    }
    fn description(&self) -> &str {
        &self.name_str
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object", "additionalProperties": true})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn scope(&self) -> ToolScope {
        ToolScope::All
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.sequence.lock().push(self.name_str.clone());
        self.calls.lock().push(args);
        Ok(self.result.clone())
    }
}

#[async_trait]
impl Tool for RecordingTool {
    fn name(&self) -> &str {
        &self.name_str
    }
    fn description(&self) -> &str {
        &self.description_str
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object", "additionalProperties": true})
    }
    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
    fn scope(&self) -> ToolScope {
        self.scope_v
    }
    fn category(&self) -> ToolCategory {
        self.category_v
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.calls.lock().push(args);
        Ok(self.result.clone())
    }
}

// ── 1. Keyword-driven loop: prompt-guided XML path ────────────────

#[tokio::test]
async fn keyword_provider_drives_prompt_guided_tool_loop_to_completion() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "search",
            ScriptedToolCall::new("search_tool", json!({"q": "rust"})),
        )
        .with_text("Looking it up."),
        KeywordRule::final_reply("search_tool-ok", "Here is the answer."),
    ]);

    let (search_tool, search_calls) = RecordingTool::echo("search_tool");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(search_tool)];

    let mut history = vec![ChatMessage::user("please search the web for rust news")];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("loop should complete");

    assert_eq!(result, "Here is the answer.");
    assert_eq!(
        search_calls.lock().len(),
        1,
        "tool should fire exactly once"
    );
    assert_eq!(search_calls.lock()[0]["q"], "rust");
    assert!(provider.turn_count() >= 2);
}

// ── 2. Keyword-driven loop: native tool_calls path ────────────────

#[tokio::test]
async fn keyword_provider_drives_native_tool_calls_path() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "weather",
            ScriptedToolCall::new("weather_tool", json!({"city": "Berlin"})),
        ),
        KeywordRule::final_reply("weather_tool-ok", "It's sunny."),
    ])
    .with_native_tools(true);

    let (weather_tool, weather_calls) = RecordingTool::echo("weather_tool");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(weather_tool)];

    let mut history = vec![ChatMessage::user("what's the weather in Berlin?")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock-native",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("loop should complete");

    assert_eq!(out, "It's sunny.");
    assert_eq!(weather_calls.lock().len(), 1);
    // History should contain a tool role message (native path) referencing the call id.
    assert!(history.iter().any(|m| m.role == "tool"));
    let tool_msg = history.iter().find(|m| m.role == "tool").unwrap();
    assert!(
        tool_msg.content.contains("weather_tool-ok"),
        "tool result should be threaded through history: {}",
        tool_msg.content
    );
}

// ── 3. Multi-tool chain via successive keyword matches ────────────

#[tokio::test]
async fn keyword_provider_chains_multiple_tools_across_iterations() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "draft an email",
            ScriptedToolCall::new("draft_tool", json!({"to": "alice@example.com"})),
        ),
        KeywordRule::tool_call(
            "draft_tool-ok",
            ScriptedToolCall::new("send_tool", json!({"draft_id": "d-1"})),
        ),
        KeywordRule::final_reply("send_tool-ok", "Email sent to alice."),
    ]);

    let (draft_tool, draft_calls) = RecordingTool::echo("draft_tool");
    let (send_tool, send_calls) = RecordingTool::echo("send_tool");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(draft_tool), Box::new(send_tool)];

    let mut history = vec![ChatMessage::user("draft an email to alice")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        10,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "Email sent to alice.");
    assert_eq!(draft_calls.lock().len(), 1);
    assert_eq!(send_calls.lock().len(), 1);
}

// ── 4. Crypto wallet flow: inspect → quote → confirm → execute ───

#[tokio::test]
async fn crypto_wallet_send_flow_sequences_wallet_tools_and_confirmation_gate() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "send john $5",
            ScriptedToolCall::new("wallet_status", json!({})),
        ),
        KeywordRule::tool_call(
            "wallet_status-ok",
            ScriptedToolCall::new("wallet_balances", json!({})),
        ),
        KeywordRule::tool_call(
            "wallet_balances-ok",
            ScriptedToolCall::new("wallet_chain_status", json!({})),
        ),
        KeywordRule::tool_call(
            "wallet_chain_status-ok",
            ScriptedToolCall::new(
                "wallet_prepare_transfer",
                json!({
                    "chain": "evm",
                    "to_address": "0x00000000000000000000000000000000000000aa",
                    "amount_raw": "5000000",
                }),
            ),
        ),
        KeywordRule::tool_call(
            "wallet_prepare_transfer-ok",
            ScriptedToolCall::new(
                "ask_user_clarification",
                json!({"question": "Send $5 to John on EVM?"}),
            ),
        ),
        KeywordRule::tool_call(
            "ask_user_clarification-ok",
            ScriptedToolCall::new(
                "wallet_execute_prepared",
                json!({"quoteId": "q_test_send_john", "confirmed": true}),
            ),
        ),
        KeywordRule::final_reply(
            "wallet_execute_prepared-ok",
            "Prepared quote confirmed and handed to the wallet signer.",
        ),
    ])
    .with_native_tools(true);

    let sequence = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let (wallet_status, wallet_status_calls) = SequencedTool::new(
        "wallet_status",
        ToolResult::success("wallet_status-ok"),
        sequence.clone(),
    );
    let (wallet_balances, wallet_balances_calls) = SequencedTool::new(
        "wallet_balances",
        ToolResult::success("wallet_balances-ok"),
        sequence.clone(),
    );
    let (wallet_chain_status, wallet_chain_status_calls) = SequencedTool::new(
        "wallet_chain_status",
        ToolResult::success("wallet_chain_status-ok"),
        sequence.clone(),
    );
    let (wallet_prepare_transfer, wallet_prepare_transfer_calls) = SequencedTool::new(
        "wallet_prepare_transfer",
        ToolResult::success("wallet_prepare_transfer-ok"),
        sequence.clone(),
    );
    let (ask_user_clarification, ask_user_clarification_calls) = SequencedTool::new(
        "ask_user_clarification",
        ToolResult::success("ask_user_clarification-ok"),
        sequence.clone(),
    );
    let (wallet_execute_prepared, wallet_execute_prepared_calls) = SequencedTool::new(
        "wallet_execute_prepared",
        ToolResult::success("wallet_execute_prepared-ok"),
        sequence.clone(),
    );

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(wallet_status),
        Box::new(wallet_balances),
        Box::new(wallet_chain_status),
        Box::new(wallet_prepare_transfer),
        Box::new(ask_user_clarification),
        Box::new(wallet_execute_prepared),
    ];
    let mut history = vec![ChatMessage::user("Please send John $5 on EVM.")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock-native",
        "test-model",
        0.0,
        true,
        None,
        "web",
        &mm(),
        10,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("crypto wallet flow should complete");

    assert_eq!(
        out,
        "Prepared quote confirmed and handed to the wallet signer."
    );
    assert_eq!(
        sequence.lock().clone(),
        vec![
            "wallet_status",
            "wallet_balances",
            "wallet_chain_status",
            "wallet_prepare_transfer",
            "ask_user_clarification",
            "wallet_execute_prepared",
        ]
    );
    assert_eq!(wallet_status_calls.lock().len(), 1);
    assert_eq!(wallet_balances_calls.lock().len(), 1);
    assert_eq!(wallet_chain_status_calls.lock().len(), 1);
    assert_eq!(
        wallet_prepare_transfer_calls.lock()[0],
        json!({
            "chain": "evm",
            "to_address": "0x00000000000000000000000000000000000000aa",
            "amount_raw": "5000000",
        })
    );
    assert_eq!(
        ask_user_clarification_calls.lock()[0]["question"],
        "Send $5 to John on EVM?"
    );
    assert_eq!(
        wallet_execute_prepared_calls.lock()[0],
        json!({"quoteId": "q_test_send_john", "confirmed": true})
    );
}

#[tokio::test]
async fn crypto_wallet_send_flow_does_not_execute_when_confirmation_is_not_granted() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "send john $5",
            ScriptedToolCall::new("wallet_status", json!({})),
        ),
        KeywordRule::tool_call(
            "wallet_status-ok",
            ScriptedToolCall::new("wallet_prepare_transfer", json!({"chain": "evm"})),
        ),
        KeywordRule::tool_call(
            "wallet_prepare_transfer-ok",
            ScriptedToolCall::new(
                "ask_user_clarification",
                json!({"question": "Confirm the transfer?"}),
            ),
        ),
        KeywordRule::final_reply(
            "user declined the transfer",
            "Cancelled before execution because the user did not confirm.",
        ),
    ])
    .with_native_tools(true);

    let sequence = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let (wallet_status, _) = SequencedTool::new(
        "wallet_status",
        ToolResult::success("wallet_status-ok"),
        sequence.clone(),
    );
    let (wallet_prepare_transfer, _) = SequencedTool::new(
        "wallet_prepare_transfer",
        ToolResult::success("wallet_prepare_transfer-ok"),
        sequence.clone(),
    );
    let (ask_user_clarification, _) = SequencedTool::new(
        "ask_user_clarification",
        ToolResult::success("user declined the transfer"),
        sequence.clone(),
    );
    let (wallet_execute_prepared, wallet_execute_prepared_calls) = SequencedTool::new(
        "wallet_execute_prepared",
        ToolResult::success("wallet_execute_prepared-ok"),
        sequence.clone(),
    );

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(wallet_status),
        Box::new(wallet_prepare_transfer),
        Box::new(ask_user_clarification),
        Box::new(wallet_execute_prepared),
    ];
    let mut history = vec![ChatMessage::user("Please send John $5 on EVM.")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock-native",
        "test-model",
        0.0,
        true,
        None,
        "telegram",
        &mm(),
        8,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("declined flow should still complete");

    assert_eq!(
        out,
        "Cancelled before execution because the user did not confirm."
    );
    assert_eq!(
        sequence.lock().clone(),
        vec![
            "wallet_status",
            "wallet_prepare_transfer",
            "ask_user_clarification",
        ]
    );
    assert!(
        wallet_execute_prepared_calls.lock().is_empty(),
        "execute tool must not run after a declined confirmation"
    );
}

#[tokio::test]
async fn keyword_provider_uses_latest_tool_result_to_drive_the_next_tool_call() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "start lookup",
            ScriptedToolCall::new("lookup_tool", json!({"symbol": "BTC"})),
        ),
        KeywordRule::tool_call(
            "lookup_tool-ok",
            ScriptedToolCall::new("enrich_tool", json!({"source": "lookup"})),
        ),
        KeywordRule::final_reply("enrich_tool-ok", "Finished after the second tool."),
    ])
    .with_native_tools(true);

    let (lookup_tool, lookup_calls) = RecordingTool::echo("lookup_tool");
    let (enrich_tool, enrich_calls) = RecordingTool::echo("enrich_tool");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(lookup_tool), Box::new(enrich_tool)];

    let mut history = vec![ChatMessage::user("please start lookup for BTC")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock-native",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        10,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("loop should complete");

    assert_eq!(out, "Finished after the second tool.");
    assert_eq!(lookup_calls.lock().as_slice(), &[json!({"symbol": "BTC"})]);
    assert_eq!(
        enrich_calls.lock().as_slice(),
        &[json!({"source": "lookup"})]
    );

    let turns = provider.turns();
    assert_eq!(
        turns.len(),
        3,
        "expected two tool turns and one final reply"
    );
    assert_eq!(turns[0].rule_keyword.as_deref(), Some("start lookup"));
    assert_eq!(turns[1].rule_keyword.as_deref(), Some("lookup_tool-ok"));
    assert_eq!(turns[2].rule_keyword.as_deref(), Some("enrich_tool-ok"));

    let second_turn_probe = turns[1]
        .messages
        .iter()
        .rev()
        .find(|msg| msg.role == "tool")
        .map(|msg| msg.content.clone())
        .unwrap_or_default();
    assert!(
        second_turn_probe.contains("lookup_tool-ok"),
        "second turn should be driven by the first tool result, got: {second_turn_probe}"
    );
}

#[tokio::test]
async fn keyword_provider_executes_multiple_native_tool_calls_from_one_turn() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule {
            keyword: "do both".to_string(),
            tool_calls: vec![
                ScriptedToolCall::new("lookup_tool", json!({"symbol": "BTC"})),
                ScriptedToolCall::new("enrich_tool", json!({"source": "coinbase"})),
            ],
            final_text: Some("Running both tools.".to_string()),
            max_fires: Some(1),
        },
        KeywordRule::final_reply("enrich_tool-ok", "Both tools completed."),
    ])
    .with_native_tools(true);

    let (lookup_tool, lookup_calls) = RecordingTool::echo("lookup_tool");
    let (enrich_tool, enrich_calls) = RecordingTool::echo("enrich_tool");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(lookup_tool), Box::new(enrich_tool)];

    let mut history = vec![ChatMessage::user("please do both actions now")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock-native",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        10,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("loop should complete");

    assert_eq!(out, "Both tools completed.");
    assert_eq!(lookup_calls.lock().as_slice(), &[json!({"symbol": "BTC"})]);
    assert_eq!(
        enrich_calls.lock().as_slice(),
        &[json!({"source": "coinbase"})]
    );

    let turns = provider.turns();
    assert_eq!(turns[0].emitted_tool_calls.len(), 2);
    assert_eq!(turns[0].emitted_tool_calls[0].name, "lookup_tool");
    assert_eq!(turns[0].emitted_tool_calls[1].name, "enrich_tool");
}

// ── 6. Unknown tool name handled gracefully ───────────────────────

#[tokio::test]
async fn keyword_provider_unknown_tool_surfaces_error_and_loop_continues() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("go", ScriptedToolCall::new("nonexistent_tool", json!({}))),
        // After we see "Unknown tool" in the role=tool injection, give up.
        KeywordRule::final_reply("unknown tool", "Sorry, I can't do that."),
    ]);

    let tools: Vec<Box<dyn Tool>> = vec![];

    let mut history = vec![ChatMessage::user("go go go")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "Sorry, I can't do that.");
    // The Tool Results message should record the "Unknown tool: nonexistent_tool".
    assert!(history.iter().any(|m| m.content.contains("Unknown tool")));
}

// ── 5. Max iterations guard ───────────────────────────────────────

#[tokio::test]
async fn run_tool_call_loop_returns_max_iterations_error() {
    // Configure a rule that keeps firing forever — but cap iterations.
    let provider = KeywordScriptedProvider::new(vec![KeywordRule {
        keyword: "echo-ok".to_string(), // matches tool result, so it loops
        tool_calls: vec![ScriptedToolCall::new("echo", json!({}))],
        final_text: None,
        max_fires: None,
    }])
    // First turn: kick it off
    .with_fallback("end");
    provider.push_forced_response(ChatResponse {
        text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
        tool_calls: vec![],
        usage: None,
    });

    let (echo_tool, _) = RecordingTool::echo("echo");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(echo_tool)];
    let mut history = vec![ChatMessage::user("loop forever")];

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        3,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect_err("should hit max iterations");

    let s = err.to_string();
    assert!(
        s.contains("3") && s.to_lowercase().contains("iteration"),
        "expected MaxIterationsExceeded with 3, got: {s}"
    );
}

// ── 6. CliRpcOnly tools are blocked in the agent loop ─────────────

struct CliOnlyTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CliOnlyTool {
    fn name(&self) -> &str {
        "cli_only_tool"
    }
    fn description(&self) -> &str {
        "cli-only"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    fn scope(&self) -> ToolScope {
        ToolScope::CliRpcOnly
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("ran"))
    }
}

#[tokio::test]
async fn agent_loop_refuses_clirpconly_tools() {
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = CliOnlyTool {
        calls: calls.clone(),
    };

    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("use", ScriptedToolCall::new("cli_only_tool", json!({}))),
        KeywordRule::final_reply("only available via", "Denied as expected."),
    ]);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(tool)];
    let mut history = vec![ChatMessage::user("use the tool")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "Denied as expected.");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "CliRpcOnly tool must never execute in the agent loop"
    );
}

// ── 7. Tool error result is threaded back as `Error: …` ───────────

struct FailingTool;

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        "fail_tool"
    }
    fn description(&self) -> &str {
        "always fails"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::error("boom"))
    }
}

#[tokio::test]
async fn tool_error_result_is_surfaced_to_next_iteration() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("try", ScriptedToolCall::new("fail_tool", json!({}))),
        KeywordRule::final_reply("boom", "got the error"),
    ]);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(FailingTool)];
    let mut history = vec![ChatMessage::user("try the broken tool")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "got the error");
    assert!(history.iter().any(|m| m.content.contains("Error: boom")));
}

// ── 8. Tool that bails with anyhow::Error ─────────────────────────

struct PanickyTool;

#[async_trait]
impl Tool for PanickyTool {
    fn name(&self) -> &str {
        "panicky"
    }
    fn description(&self) -> &str {
        "raises anyhow"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        anyhow::bail!("kaboom")
    }
}

#[tokio::test]
async fn tool_anyhow_error_surfaces_in_history() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("run", ScriptedToolCall::new("panicky", json!({}))),
        KeywordRule::final_reply("kaboom", "tool blew up"),
    ]);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(PanickyTool)];
    let mut history = vec![ChatMessage::user("run it")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "tool blew up");
    assert!(history.iter().any(|m| m.content.contains("kaboom")));
}

// ── 9. visible_tool_names whitelist hides tools from the model ────

#[tokio::test]
async fn visible_tool_names_whitelist_rejects_filtered_out_tools() {
    let provider = KeywordScriptedProvider::new(vec![
        // Model asks for a tool that *exists* but is filtered out.
        KeywordRule::tool_call("go", ScriptedToolCall::new("hidden", json!({}))),
        KeywordRule::final_reply("unknown tool", "Cannot reach hidden tool."),
    ]);

    let (visible_tool, visible_calls) = RecordingTool::echo("visible");
    let (hidden_tool, hidden_calls) = RecordingTool::echo("hidden");
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(visible_tool), Box::new(hidden_tool)];

    let mut visible = std::collections::HashSet::new();
    visible.insert("visible".to_string());

    let mut history = vec![ChatMessage::user("go please")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        Some(&visible),
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "Cannot reach hidden tool.");
    assert_eq!(visible_calls.lock().len(), 0);
    assert_eq!(
        hidden_calls.lock().len(),
        0,
        "hidden tool must not execute when filtered out"
    );
}

// ── 10. extra_tools are reachable alongside the registry ──────────

#[tokio::test]
async fn extra_tools_are_invokable_alongside_registry() {
    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call("delegate", ScriptedToolCall::new("extra", json!({"x": 1}))),
        KeywordRule::final_reply("extra-ok", "delegated"),
    ]);

    let (extra_tool, extra_calls) = RecordingTool::echo("extra");
    let extras: Vec<Box<dyn Tool>> = vec![Box::new(extra_tool)];

    let tools: Vec<Box<dyn Tool>> = vec![];
    let mut history = vec![ChatMessage::user("delegate the work")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &extras,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "delegated");
    assert_eq!(extra_calls.lock().len(), 1);
}

// ── 11. Composio fixture backend: end-to-end client wiring ────────

#[tokio::test]
async fn fake_composio_backend_serves_realistic_toolkits() {
    let backend = spawn_fake_composio_backend(ComposioFixture::realistic()).await;
    let client = backend.client();

    let toolkits = client.list_toolkits().await.unwrap();
    assert!(toolkits.toolkits.contains(&"gmail".to_string()));
    assert!(toolkits.toolkits.contains(&"github".to_string()));

    let conns = client.list_connections().await.unwrap();
    assert!(
        conns.connections.iter().any(|c| c.toolkit == "gmail"),
        "expected a gmail connection: {:?}",
        conns.connections
    );

    let exec = client
        .execute_tool(
            "GMAIL_SEND_EMAIL",
            Some(json!({"recipient_email": "a@b.com", "subject": "hi", "body": "hello"})),
        )
        .await
        .unwrap();
    assert!(exec.successful, "execute should report success");
    let resp_json = serde_json::to_value(&exec.data).unwrap();
    assert!(
        resp_json.to_string().contains("gmail-msg-1234"),
        "expected fixture response, got: {resp_json}"
    );

    let reqs = backend.requests();
    // Authorize wasn't called; toolkits + connections + execute were.
    assert!(reqs.iter().any(|(_, p, _)| p == "/toolkits"));
    assert!(reqs.iter().any(|(_, p, _)| p == "/connections"));
    assert!(reqs.iter().any(|(_, p, _)| p == "/execute"));
}

#[tokio::test]
async fn fake_composio_backend_can_match_execute_rules_by_argument_content() {
    let mut fixture = ComposioFixture::realistic();
    fixture.execute_rules.push(
        ComposioExecuteRule::new(
            "GMAIL_FETCH_EMAILS",
            json!({
                "messages": [
                    {
                        "id": "gmail-priority-1",
                        "subject": "Release blocker",
                        "snippet": "The release blocker is the broken memory recall spec."
                    }
                ]
            }),
        )
        .when_argument_contains("arguments.query", "release blocker"),
    );
    let backend = spawn_fake_composio_backend(fixture).await;
    let client = backend.client();

    let exec = client
        .execute_tool(
            "GMAIL_FETCH_EMAILS",
            Some(json!({
                "query": "label:inbox release blocker",
                "max_results": 5
            })),
        )
        .await
        .unwrap();

    assert!(exec.successful, "execute should report success");
    let resp_json = serde_json::to_value(&exec.data).unwrap();
    assert!(
        resp_json.to_string().contains("gmail-priority-1"),
        "expected rule-driven response, got: {resp_json}"
    );
    let reqs = backend.requests();
    let exec_req = reqs
        .iter()
        .find(|(method, path, _)| method == "POST" && path == "/execute")
        .expect("execute request should be recorded");
    assert_eq!(
        exec_req.2["arguments"]["query"],
        "label:inbox release blocker"
    );
}

// ── 12. End-to-end: harness drives a Composio tool against fake backend

#[tokio::test]
async fn harness_invokes_composio_action_tool_against_fake_backend() {
    use crate::openhuman::composio::ComposioActionTool;
    use crate::openhuman::config::TEST_ENV_LOCK;

    // Post-#1710-Wave-4, `ComposioActionTool::execute` reloads config via
    // `load_config_with_timeout()` per call, so the injected `Arc<Config>`
    // only routes to the fake backend if it is the live on-disk config.
    // Hold `TEST_ENV_LOCK` and point `OPENHUMAN_WORKSPACE` at the
    // persisted fake-backend workspace.
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let backend = spawn_fake_composio_backend(ComposioFixture::realistic()).await;
    let (config, workspace_root) = backend.config_persisted().await;
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", &workspace_root);
    }

    let tool = ComposioActionTool::new(
        config,
        "GMAIL_SEND_EMAIL".to_string(),
        "Send a Gmail email".to_string(),
        Some(json!({"type": "object"})),
    );

    let provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "send email",
            ScriptedToolCall::new(
                "GMAIL_SEND_EMAIL",
                json!({"recipient_email": "alice@example.com", "subject": "hi", "body": "hello"}),
            ),
        ),
        KeywordRule::final_reply("gmail-msg-1234", "Email sent."),
    ]);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(tool)];
    let mut history = vec![ChatMessage::user("send email to alice")];

    let out = run_tool_call_loop(
        &provider,
        &mut history,
        &tools,
        "mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(out, "Email sent.");
    // Backend should have received the execute POST.
    let reqs = backend.requests();
    let exec = reqs
        .iter()
        .find(|(m, p, _)| m == "POST" && p == "/execute")
        .expect("execute call must have hit the backend");
    assert_eq!(exec.2["tool"], "GMAIL_SEND_EMAIL");
    assert_eq!(exec.2["arguments"]["recipient_email"], "alice@example.com");

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

// ── 13. Orchestrator-prompt → delegation → Composio round-trip ────
//
// End-to-end test that ties together the three things the user needs
// confidence in:
//
//   1. The orchestrator's system prompt (built by
//      `orchestrator::prompt::build`) advertises a connected toolkit
//      via the collapsed `delegate_to_integrations_agent` tool.
//   2. Given that prompt and a user task that mentions the toolkit,
//      the LLM (mocked) emits a tool call that satisfies the real
//      `SkillDelegationTool` schema.
//   3. Delegation reaches the integrations side, where a
//      `ComposioActionTool` is dispatched against a real
//      `ComposioClient` pointed at a hermetic fake backend — and the
//      backend records the action with the orchestrator-provided args.
//
// To avoid pulling in the full sub-agent runner, the test substitutes
// a `TestDelegationTool` that mirrors `SkillDelegationTool`'s contract
// (same tool name, same schema validation against the connected
// toolkit list) but runs a *nested* `run_tool_call_loop` for the
// integrations side instead of calling `dispatch_subagent`. The
// nested loop is the same code path the real integrations_agent uses,
// so the wiring under test is the orchestrator → delegation arg →
// integrations LLM → ComposioActionTool → backend chain.

struct TestDelegationTool {
    connected_toolkits: Vec<(String, String)>,
    nested_tools: Arc<parking_lot::Mutex<Option<Vec<Box<dyn Tool>>>>>,
    inner_provider: Arc<KeywordScriptedProvider>,
}

impl TestDelegationTool {
    fn new(
        connected_toolkits: Vec<(String, String)>,
        nested_tools: Vec<Box<dyn Tool>>,
        inner_provider: Arc<KeywordScriptedProvider>,
    ) -> Self {
        Self {
            connected_toolkits,
            nested_tools: Arc::new(parking_lot::Mutex::new(Some(nested_tools))),
            inner_provider,
        }
    }
}

#[async_trait]
impl Tool for TestDelegationTool {
    fn name(&self) -> &str {
        // Mirror SkillDelegationTool's canonical name so the orchestrator
        // system prompt's references resolve.
        "delegate_to_integrations_agent"
    }
    fn description(&self) -> &str {
        "Delegate to integrations_agent (test stand-in for SkillDelegationTool)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        // Same shape as the real SkillDelegationTool.
        let slugs: Vec<&str> = self
            .connected_toolkits
            .iter()
            .map(|(s, _)| s.as_str())
            .collect();
        json!({
            "type": "object",
            "required": ["toolkit", "prompt"],
            "properties": {
                "toolkit": {"type": "string", "enum": slugs},
                "prompt": {"type": "string"}
            }
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let toolkit = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if toolkit.is_empty() {
            return Ok(ToolResult::error("`toolkit` is required"));
        }
        let known = self
            .connected_toolkits
            .iter()
            .any(|(slug, _)| slug == &toolkit);
        if !known {
            return Ok(ToolResult::error(format!(
                "toolkit `{toolkit}` is not connected"
            )));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error("`prompt` is required"));
        }

        // Take ownership of the nested tool list (one-shot).
        let nested_tools = self
            .nested_tools
            .lock()
            .take()
            .expect("nested tools already consumed");

        // Run a NESTED tool loop — same code path the integrations_agent
        // uses inside the real sub-agent runner.
        let mut nested_history = vec![ChatMessage::user(format!("[toolkit={toolkit}] {prompt}"))];
        let out = run_tool_call_loop(
            self.inner_provider.as_ref(),
            &mut nested_history,
            &nested_tools,
            "test-integrations",
            "test-model",
            0.0,
            true,
            None,
            "channel",
            &mm(),
            5,
            None,
            None,
            &[],
            None,
            None,
        )
        .await?;

        Ok(ToolResult::success(out))
    }
}

#[tokio::test]
async fn orchestrator_prompt_drives_composio_call_via_delegation_chain() {
    use crate::openhuman::agent::agents::orchestrator::prompt as orch_prompt;
    use crate::openhuman::agent::prompts::types::ConnectedIntegration;
    use crate::openhuman::composio::ComposioActionTool;
    use crate::openhuman::config::TEST_ENV_LOCK;

    // Post-#1710-Wave-4, `ComposioActionTool::execute` reloads config via
    // `load_config_with_timeout()` per call. Hold `TEST_ENV_LOCK` and
    // point `OPENHUMAN_WORKSPACE` at the persisted fake-backend
    // workspace so the tool routes to the fake backend.
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // ── 1. Build the orchestrator's system prompt with gmail wired in.
    let integrations = vec![ConnectedIntegration {
        toolkit: "gmail".into(),
        description: "Email send/fetch via Gmail.".into(),
        tools: Vec::new(),
        connected: true,
    }];
    let ctx = {
        use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
        use std::collections::HashSet;
        use std::sync::OnceLock;
        static EMPTY: OnceLock<HashSet<String>> = OnceLock::new();
        crate::openhuman::context::prompt::PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "orchestrator",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: EMPTY.get_or_init(HashSet::new),
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &integrations,
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
        }
    };
    let system_prompt = orch_prompt::build(&ctx).expect("build orchestrator prompt");
    // The prompt must explicitly route gmail tasks via the collapsed
    // delegation tool — otherwise the orchestrator can't know to call it.
    assert!(system_prompt.contains("## Connected Integrations"));
    assert!(system_prompt.contains("delegate_to_integrations_agent"));
    assert!(system_prompt.contains("toolkit: \"gmail\""));

    // ── 2. Spawn the fake Composio backend + wire a ComposioActionTool.
    let backend = spawn_fake_composio_backend(ComposioFixture::realistic()).await;
    let (composio_config, workspace_root) = backend.config_persisted().await;
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", &workspace_root);
    }
    let gmail_action_tool: Box<dyn Tool> = Box::new(ComposioActionTool::new(
        composio_config,
        "GMAIL_SEND_EMAIL".to_string(),
        "Send a Gmail email".to_string(),
        Some(json!({"type": "object"})),
    ));

    // ── 3. Inner (integrations-side) provider: emit a ComposioActionTool call,
    // then a final reply once it sees the action's success marker.
    let inner_provider = Arc::new(KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "toolkit=gmail",
            ScriptedToolCall::new(
                "GMAIL_SEND_EMAIL",
                json!({
                    "recipient_email": "alice@example.com",
                    "subject": "hi",
                    "body": "hello from orchestrator",
                }),
            ),
        ),
        KeywordRule::final_reply("gmail-msg-1234", "delivered"),
    ]));

    let nested_tools: Vec<Box<dyn Tool>> = vec![gmail_action_tool];

    // ── 4. Outer (orchestrator) provider: when the user asks to email Alice
    // via gmail, emit the delegation tool call. After the delegation
    // returns "delivered", produce the final user-facing reply.
    let outer_provider = KeywordScriptedProvider::new(vec![
        KeywordRule::tool_call(
            "send an email to alice",
            ScriptedToolCall::new(
                "delegate_to_integrations_agent",
                json!({
                    "toolkit": "gmail",
                    "prompt": "Send an email to alice@example.com saying hi"
                }),
            ),
        ),
        KeywordRule::final_reply("delivered", "I've sent the email to Alice."),
    ]);

    // ── 5. Wire the test delegation tool that bridges outer -> inner loop.
    let delegation_tool: Box<dyn Tool> = Box::new(TestDelegationTool::new(
        vec![(
            "gmail".to_string(),
            "Email send/fetch via Gmail.".to_string(),
        )],
        nested_tools,
        inner_provider.clone(),
    ));
    let outer_tools: Vec<Box<dyn Tool>> = vec![delegation_tool];

    let mut history = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user("Please send an email to alice@example.com saying hi via gmail."),
    ];

    // ── 6. Drive the outer (orchestrator) loop.
    let final_reply = run_tool_call_loop(
        &outer_provider,
        &mut history,
        &outer_tools,
        "orchestrator-mock",
        "test-model",
        0.0,
        true,
        None,
        "channel",
        &mm(),
        5,
        None,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("orchestrator loop should complete");

    assert_eq!(final_reply, "I've sent the email to Alice.");

    // ── 7. Assert the Composio backend actually received the action with
    // the orchestrator-routed arguments.
    let backend_reqs = backend.requests();
    let exec = backend_reqs
        .iter()
        .find(|(m, p, _)| m == "POST" && p == "/execute")
        .expect("Composio /execute must have been called via the orchestrator chain");
    assert_eq!(exec.2["tool"], "GMAIL_SEND_EMAIL");
    assert_eq!(exec.2["arguments"]["recipient_email"], "alice@example.com");
    assert_eq!(exec.2["arguments"]["subject"], "hi");

    // ── 8. Both sides of the chain should have seen exactly one turn that
    // emitted the expected call.
    let outer_turns = outer_provider.turns();
    assert!(
        outer_turns
            .iter()
            .any(|t| t.rule_keyword.as_deref() == Some("send an email to alice")),
        "orchestrator must have matched the delegation rule"
    );
    let inner_turns = inner_provider.turns();
    assert!(
        inner_turns
            .iter()
            .any(|t| t.rule_keyword.as_deref() == Some("toolkit=gmail")),
        "integrations agent must have matched its tool-call rule"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
