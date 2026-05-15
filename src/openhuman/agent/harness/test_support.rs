//! Smart-mock test support for the agent harness.
//!
//! This module provides a reusable "fake LLM" that drives the real
//! [`run_tool_call_loop`] without needing any network access. Two
//! building blocks are exposed:
//!
//! 1. [`KeywordScriptedProvider`] — a [`Provider`] implementation that
//!    inspects the latest `user` message of the conversation and emits
//!    canned tool calls (or a final reply) when a configured keyword
//!    matches. The first turn that has *no* matching rule returns a
//!    plain "done" reply, which terminates the loop deterministically.
//!
//!    Compared to the hand-rolled `ScriptedProvider` in
//!    [`super::tool_loop_tests`], this provider:
//!      * Reacts to the rolling conversation state instead of replaying
//!        a fixed queue — so tests can exercise iterative loops where
//!        what the LLM does next depends on what tools returned.
//!      * Supports both **native** OpenAI-style `tool_calls` and
//!        **prompt-guided** `<tool_call>…</tool_call>` text — flipping
//!        a single flag toggles which surface the harness exercises.
//!      * Records the messages it saw and the responses it returned for
//!        post-hoc assertion.
//!
//! 2. [`spawn_fake_composio_backend`] — boots a minimal axum app that
//!    responds to the `/agent-integrations/composio/*` routes with
//!    realistic-looking fixture data (real-world toolkit/action shapes,
//!    not synthetic gibberish). Tests can pair this with a
//!    [`crate::openhuman::composio::ComposioClient`] to exercise the
//!    full agent → tool → backend → response flow against a hermetic
//!    in-process server.
//!
//! Both helpers are `#[cfg(test)]`-only so they never leak into release
//! binaries.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;

use crate::openhuman::providers::traits::ProviderCapabilities;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall};

/// One scripted reaction the [`KeywordScriptedProvider`] can emit when
/// it sees its keyword in the latest user/tool turn.
#[derive(Debug, Clone)]
pub struct KeywordRule {
    /// Substring matched (case-insensitive) against the latest user
    /// or tool message in the conversation.
    pub keyword: String,
    /// Tool calls to emit. Empty ⇒ no tool calls, only `final_text`.
    pub tool_calls: Vec<ScriptedToolCall>,
    /// Optional plain-text body to include alongside any tool calls.
    /// When `tool_calls` is empty, this becomes the loop-terminating
    /// final response.
    pub final_text: Option<String>,
    /// How many times this rule may fire. `None` ⇒ unlimited.
    pub max_fires: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ScriptedToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ScriptedToolCall {
    pub fn new(name: impl Into<String>, arguments: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            arguments,
        }
    }
}

impl KeywordRule {
    pub fn final_reply(keyword: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            keyword: keyword.into(),
            tool_calls: Vec::new(),
            final_text: Some(text.into()),
            max_fires: None,
        }
    }

    pub fn tool_call(keyword: impl Into<String>, call: ScriptedToolCall) -> Self {
        Self {
            keyword: keyword.into(),
            tool_calls: vec![call],
            final_text: None,
            max_fires: Some(1),
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.final_text = Some(text.into());
        self
    }

    pub fn unlimited(mut self) -> Self {
        self.max_fires = None;
        self
    }
}

/// Snapshot of one turn the provider served — handy for tests that
/// want to assert what the LLM "saw" without coupling to the harness
/// internals.
#[derive(Debug, Clone)]
pub struct ProviderTurn {
    pub messages: Vec<ChatMessage>,
    pub rule_keyword: Option<String>,
    pub emitted_tool_calls: Vec<ToolCall>,
    pub emitted_text: Option<String>,
}

struct ProviderState {
    rules: Vec<KeywordRule>,
    fired: Vec<usize>,
    turns: Vec<ProviderTurn>,
    fallback_text: String,
    next_call_id: usize,
    /// Optional queue of scripted responses to consume *before* the
    /// keyword rules run — useful when a test wants the first turn to
    /// behave deterministically regardless of the user message.
    forced: VecDeque<ChatResponse>,
}

/// Smart provider that reacts to conversation state via keyword rules.
pub struct KeywordScriptedProvider {
    state: Arc<Mutex<ProviderState>>,
    native_tools: bool,
    vision: bool,
}

impl KeywordScriptedProvider {
    pub fn new(rules: Vec<KeywordRule>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProviderState {
                rules,
                fired: Vec::new(),
                turns: Vec::new(),
                fallback_text: "done".to_string(),
                next_call_id: 0,
                forced: VecDeque::new(),
            })),
            native_tools: false,
            vision: false,
        }
    }

    pub fn with_native_tools(mut self, enabled: bool) -> Self {
        self.native_tools = enabled;
        self
    }

    pub fn with_vision(mut self, enabled: bool) -> Self {
        self.vision = enabled;
        self
    }

    pub fn with_fallback(self, text: impl Into<String>) -> Self {
        {
            let mut guard = self.state.lock();
            guard.fallback_text = text.into();
        }
        self
    }

    pub fn push_forced_response(&self, resp: ChatResponse) {
        self.state.lock().forced.push_back(resp);
    }

    pub fn turns(&self) -> Vec<ProviderTurn> {
        self.state.lock().turns.clone()
    }

    pub fn turn_count(&self) -> usize {
        self.state.lock().turns.len()
    }
}

fn latest_user_or_tool_msg(messages: &[ChatMessage]) -> Option<&ChatMessage> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user" || m.role == "tool")
}

#[async_trait]
impl Provider for KeywordScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: self.native_tools,
            vision: self.vision,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("fallback".into())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let messages = request.messages.to_vec();
        let mut state = self.state.lock();

        // Forced queue wins, regardless of keyword matching.
        if let Some(resp) = state.forced.pop_front() {
            state.turns.push(ProviderTurn {
                messages,
                rule_keyword: None,
                emitted_tool_calls: resp.tool_calls.clone(),
                emitted_text: resp.text.clone(),
            });
            return Ok(resp);
        }

        let probe = latest_user_or_tool_msg(&messages)
            .map(|m| m.content.to_lowercase())
            .unwrap_or_default();

        let mut chosen: Option<usize> = None;
        for (idx, rule) in state.rules.iter().enumerate() {
            let fired = *state.fired.get(idx).unwrap_or(&0);
            if let Some(cap) = rule.max_fires {
                if fired >= cap {
                    continue;
                }
            }
            if probe.contains(&rule.keyword.to_lowercase()) {
                chosen = Some(idx);
                break;
            }
        }

        let (rule_keyword, tool_calls, text) = if let Some(idx) = chosen {
            while state.fired.len() <= idx {
                state.fired.push(0);
            }
            state.fired[idx] += 1;
            let rule = state.rules[idx].clone();
            let tool_calls: Vec<ToolCall> = if self.native_tools {
                rule.tool_calls
                    .iter()
                    .map(|c| {
                        let id = state.next_call_id;
                        state.next_call_id += 1;
                        ToolCall {
                            id: format!("call_{id}"),
                            name: c.name.clone(),
                            arguments: c.arguments.to_string(),
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let text = if self.native_tools {
                rule.final_text.clone()
            } else if !rule.tool_calls.is_empty() {
                // Prompt-guided: emit XML-wrapped tool calls in text.
                let mut body = String::new();
                if let Some(prefix) = &rule.final_text {
                    body.push_str(prefix);
                    if !prefix.ends_with('\n') {
                        body.push('\n');
                    }
                }
                for c in &rule.tool_calls {
                    body.push_str("<tool_call>");
                    body.push_str(&json!({"name": c.name, "arguments": c.arguments}).to_string());
                    body.push_str("</tool_call>\n");
                }
                Some(body)
            } else {
                rule.final_text.clone()
            };

            (Some(rule.keyword.clone()), tool_calls, text)
        } else {
            // No rule matched — emit the fallback as the final reply
            // so the loop terminates rather than hanging.
            (None, Vec::new(), Some(state.fallback_text.clone()))
        };

        let resp = ChatResponse {
            text: text.clone(),
            tool_calls: tool_calls.clone(),
            usage: None,
        };

        state.turns.push(ProviderTurn {
            messages,
            rule_keyword,
            emitted_tool_calls: tool_calls,
            emitted_text: text,
        });

        Ok(resp)
    }
}

// ── Fake Composio backend ──────────────────────────────────────────

use crate::openhuman::composio::ComposioClient;
use crate::openhuman::integrations::IntegrationClient;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};

/// Realistic-looking Composio fixture data (toolkit slugs, action
/// names, and parameter shapes lifted from the production catalog —
/// just enough to exercise the schemas without coupling tests to the
/// upstream API).
#[derive(Default, Clone)]
pub struct ComposioFixture {
    pub toolkits: Vec<String>,
    pub connections: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    /// Per-action canned execute responses, keyed by action slug.
    pub execute_responses: std::collections::HashMap<String, serde_json::Value>,
}

impl ComposioFixture {
    /// A reasonable default fixture: GMail/Notion/GitHub connections
    /// with two actions each. Use this when a test just needs *some*
    /// Composio data and doesn't care about the exact shape.
    pub fn realistic() -> Self {
        Self {
            toolkits: vec![
                "gmail".to_string(),
                "notion".to_string(),
                "github".to_string(),
                "slack".to_string(),
            ],
            connections: vec![
                json!({
                    "id": "conn_gmail_1",
                    "toolkit": "gmail",
                    "status": "ACTIVE",
                    "createdAt": "2026-04-01T12:00:00Z",
                }),
                json!({
                    "id": "conn_notion_1",
                    "toolkit": "notion",
                    "status": "ACTIVE",
                    "createdAt": "2026-04-02T08:00:00Z",
                }),
                json!({
                    "id": "conn_github_1",
                    "toolkit": "github",
                    "status": "ACTIVE",
                    "createdAt": "2026-04-03T15:30:00Z",
                }),
            ],
            tools: vec![
                json!({
                    "name": "GMAIL_SEND_EMAIL",
                    "description": "Send an email via Gmail",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "recipient_email": {"type": "string"},
                            "subject": {"type": "string"},
                            "body": {"type": "string"},
                        },
                        "required": ["recipient_email", "subject", "body"],
                    },
                }),
                json!({
                    "name": "GMAIL_FETCH_EMAILS",
                    "description": "Fetch emails from Gmail",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"},
                            "max_results": {"type": "integer"},
                        },
                    },
                }),
                json!({
                    "name": "NOTION_CREATE_PAGE",
                    "description": "Create a new Notion page",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "parent_id": {"type": "string"},
                            "title": {"type": "string"},
                        },
                        "required": ["parent_id", "title"],
                    },
                }),
                json!({
                    "name": "GITHUB_CREATE_ISSUE",
                    "description": "Open a GitHub issue",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "owner": {"type": "string"},
                            "repo": {"type": "string"},
                            "title": {"type": "string"},
                            "body": {"type": "string"},
                        },
                        "required": ["owner", "repo", "title"],
                    },
                }),
            ],
            execute_responses: [
                (
                    "GMAIL_SEND_EMAIL".to_string(),
                    json!({"message_id": "gmail-msg-1234", "thread_id": "gmail-thread-9999"}),
                ),
                (
                    "GMAIL_FETCH_EMAILS".to_string(),
                    json!({
                        "messages": [
                            {"id": "m1", "subject": "Welcome", "from": "team@openhuman.com"},
                            {"id": "m2", "subject": "Invoice", "from": "billing@stripe.com"},
                        ]
                    }),
                ),
                (
                    "NOTION_CREATE_PAGE".to_string(),
                    json!({"page_id": "notion-page-abc123", "url": "https://notion.so/abc123"}),
                ),
                (
                    "GITHUB_CREATE_ISSUE".to_string(),
                    json!({
                        "number": 42,
                        "html_url": "https://github.com/example/repo/issues/42",
                    }),
                ),
            ]
            .into_iter()
            .collect(),
        }
    }
}

#[derive(Clone)]
struct FakeComposioState {
    fixture: Arc<Mutex<ComposioFixture>>,
    requests: Arc<Mutex<Vec<(String, String, serde_json::Value)>>>,
}

/// Handle to a spawned in-process Composio backend.
pub struct FakeComposioBackend {
    pub base_url: String,
    state: FakeComposioState,
}

impl FakeComposioBackend {
    /// All `(method, path, body)` requests received in order.
    pub fn requests(&self) -> Vec<(String, String, serde_json::Value)> {
        self.state.requests.lock().clone()
    }

    /// Build a `ComposioClient` pointed at this backend.
    pub fn client(&self) -> ComposioClient {
        let inner = Arc::new(IntegrationClient::new(
            self.base_url.clone(),
            "test-token".into(),
        ));
        ComposioClient::new(inner)
    }
}

async fn record<B: serde::Serialize + Clone>(
    requests: &Arc<Mutex<Vec<(String, String, serde_json::Value)>>>,
    method: &str,
    path: &str,
    body: Option<&B>,
) {
    let body_v = match body {
        Some(b) => serde_json::to_value(b).unwrap_or(serde_json::Value::Null),
        None => serde_json::Value::Null,
    };
    requests
        .lock()
        .push((method.to_string(), path.to_string(), body_v));
}

/// Spawn an in-process Composio backend on `127.0.0.1:0`.
pub async fn spawn_fake_composio_backend(fixture: ComposioFixture) -> FakeComposioBackend {
    let state = FakeComposioState {
        fixture: Arc::new(Mutex::new(fixture)),
        requests: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get({
                let st = state.clone();
                move || async move {
                    record::<()>(&st.requests, "GET", "/toolkits", None).await;
                    let toolkits = st.fixture.lock().toolkits.clone();
                    Json(json!({
                        "success": true,
                        "data": { "toolkits": toolkits }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get({
                let st = state.clone();
                move || async move {
                    record::<()>(&st.requests, "GET", "/connections", None).await;
                    let connections = st.fixture.lock().connections.clone();
                    Json(json!({
                        "success": true,
                        "data": { "connections": connections }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get({
                let st = state.clone();
                move || async move {
                    record::<()>(&st.requests, "GET", "/tools", None).await;
                    let tools = st.fixture.lock().tools.clone();
                    Json(json!({
                        "success": true,
                        "data": { "tools": tools }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/authorize",
            post({
                let st = state.clone();
                move |Json(body): Json<serde_json::Value>| async move {
                    record(&st.requests, "POST", "/authorize", Some(&body)).await;
                    let toolkit = body
                        .get("toolkit")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    Json(json!({
                        "success": true,
                        "data": {
                            "connectUrl": format!("https://composio.dev/auth/{toolkit}"),
                            "connectionId": format!("conn_{toolkit}_pending"),
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/execute",
            post({
                let st = state.clone();
                move |Json(body): Json<serde_json::Value>| async move {
                    record(&st.requests, "POST", "/execute", Some(&body)).await;
                    // ComposioClient::execute_tool sends `{tool, arguments}`.
                    let action = body
                        .get("tool")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let fx = st.fixture.lock();
                    let response = fx
                        .execute_responses
                        .get(&action)
                        .cloned()
                        .unwrap_or_else(|| json!({"ok": true, "action": action.clone()}));
                    // Wrap in the BackendResponse envelope expected by
                    // IntegrationClient, with the inner shape matching
                    // ComposioExecuteResponse.
                    Json(json!({
                        "success": true,
                        "data": {
                            "data": response,
                            "successful": true,
                            "costUsd": 0.0,
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            delete({
                let st = state.clone();
                move |Path(id): Path<String>| async move {
                    record::<()>(&st.requests, "DELETE", &format!("/connections/{id}"), None).await;
                    let mut fx = st.fixture.lock();
                    fx.connections
                        .retain(|c| c.get("id").and_then(|v| v.as_str()).unwrap_or("") != id);
                    Json(json!({
                        "success": true,
                        "data": {"deleted": true}
                    }))
                }
            }),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    FakeComposioBackend {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        state,
    }
}

// A handler signature placeholder to silence unused warnings when the
// State extractor isn't reached (e.g. when only sub-routes fire).
#[allow(dead_code)]
async fn _unused(_state: State<FakeComposioState>) {}
