//! A `HostOnly` agent sees the host's tools and nothing else.
//!
//! A document-analysis host supplies read-only tools and confines the agent
//! to that catalog: no shell, writes, network, memory, skills, MCP or delegation.
//! Assertions inspect provider requests and attempted filesystem effects.

#![allow(dead_code)] // Each test target includes the shared scenarios but runs one case.

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{chat_completion, chat_requests, offline_config, runtime, stub_backend, tool_names};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, HostTurnTools, Provider, Runtime, Tool, ToolPolicy,
    ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

// Runtime is process-wide; tests in this file take turns.
static RUNTIME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const BARE_PROMPT: &str = "Analyze documents with the tools you have.";

/// A read-only host tool that counts its calls.
struct HostTool {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for HostTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Read-only document access supplied by the host"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    fn policy(&self) -> ToolPolicy {
        ToolPolicy::read_only()
    }
    async fn execute(&self, _: Value) -> anyhow::Result<openhuman_embed::ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(openhuman_embed::ToolResult::success(format!(
            "{}-host-result",
            self.name
        )))
    }
}

/// Answers each chat request with the next scripted body; the last repeats.
struct Script {
    bodies: Vec<Value>,
    next: AtomicUsize,
}

impl Respond for Script {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let index = self.next.fetch_add(1, Ordering::SeqCst);
        let body = self.bodies[index.min(self.bodies.len() - 1)].clone();
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// One assistant message calling every `(name, arguments)` pair at once.
fn tool_calls(calls: &[(&str, Value)]) -> Value {
    let calls: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(index, (name, arguments))| {
            json!({
                "id": format!("call_{index}"),
                "type": "function",
                "function": { "name": name, "arguments": arguments.to_string() }
            })
        })
        .collect();
    json!({
        "id": "chatcmpl-host-only",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": "fixture",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": null, "tool_calls": calls },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

async fn scripted_provider(bodies: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(Script {
            bodies,
            next: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;
    server
}

fn host_only_spec(id: &str, provider: &MockServer, calls: Arc<AtomicUsize>) -> AgentSpec {
    AgentSpec::new(id)
        .provider(
            Provider::openai_compatible(format!("{}/v1", provider.uri()), "fixture")
                .model("fixture"),
        )
        // Supervised: the tier that would park a write for approval. HostOnly
        // must not care what the host passed here.
        .access(Access::default())
        .definition(
            AgentDefinitionSpec::new()
                .bare_prompt(BARE_PROMPT)
                .tools(ToolScopeSpec::HostOnly),
        )
        .tools(move |_| {
            let calls = calls.clone();
            Box::pin(async move {
                HostTurnTools::advertised(vec![
                    Box::new(HostTool {
                        name: "read_file",
                        calls: calls.clone(),
                    }),
                    Box::new(HostTool {
                        name: "find_documents",
                        calls,
                    }),
                ])
            })
        })
}

fn system_text(request: &Request) -> String {
    let body: Value = serde_json::from_slice(&request.body).expect("json body");
    body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|m| m["role"] == "system")
        .map(|m| match &m["content"] {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn host_only_advertises_exactly_the_host_tools_under_a_bare_prompt() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = scripted_provider(vec![chat_completion("complete")]).await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .unwrap_or_else(|error| {
                    eprintln!("host-only advertise runtime build failed: {error:?}");
                    panic!("runtime");
                });
            let calls = Arc::new(AtomicUsize::new(0));
            let agent = runtime
                .agent(host_only_spec("analyst", &provider, calls))
                .expect("agent");

            agent.run("Analyze this document.").await.expect("turn");

            let requests = chat_requests(&provider).await;
            assert_eq!(requests.len(), 1);
            let mut advertised = tool_names(&requests[0]);
            advertised.sort();
            assert_eq!(advertised, vec!["find_documents", "read_file"]);
            assert_eq!(system_text(&requests[0]), BARE_PROMPT);
            runtime.remove_agent("analyst").await.unwrap();
            drop(agent);
        })
        .await
        .expect("test task");
    });
}

pub fn host_only_refuses_builtin_tools_the_model_names() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = scripted_provider(vec![
                tool_calls(&[
                    (
                        "shell",
                        json!({"command": "printf shell-ran > shell-ran.txt"}),
                    ),
                    (
                        "write_file",
                        json!({"path": "written.txt", "content": "write-ran"}),
                    ),
                    ("read_file", json!({"path": "README.md"})),
                ]),
                chat_completion("done"),
            ])
            .await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .unwrap_or_else(|error| {
                    eprintln!("host-only refusal runtime build failed: {error:?}");
                    panic!("runtime");
                });
            let calls = Arc::new(AtomicUsize::new(0));
            let agent = runtime
                .agent(host_only_spec("refuser", &provider, calls.clone()))
                .expect("agent");

            let _ = agent.run("Analyze this document.").await;

            let action_dir = agent.action_dir().to_path_buf();
            assert!(!action_dir.join("shell-ran.txt").exists(), "shell executed");
            assert!(
                !action_dir.join("written.txt").exists(),
                "write_file executed"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "the host read_file runs once"
            );

            let requests = chat_requests(&provider).await;
            assert_eq!(requests.len(), 2);
            let results = common::tool_results(&requests[1]);
            // The refusal echoes the arguments back to the model, so the
            // filesystem above is the proof nothing ran; here, that each
            // built-in was refused by name and the host tool answered.
            assert!(results.contains("read_file-host-result"), "{results}");
            assert!(results.contains("unknown tool `shell`"), "{results}");
            assert!(results.contains("unknown tool `write_file`"), "{results}");
            runtime.remove_agent("refuser").await.unwrap();
            drop(agent);
        })
        .await
        .expect("test task");
    });
}

pub fn host_only_forces_read_only_access_and_sandbox() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = scripted_provider(vec![chat_completion("ok")]).await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .unwrap_or_else(|error| {
                    eprintln!("host-only readonly runtime build failed: {error:?}");
                    panic!("runtime");
                });
            let agent = runtime
                .agent(host_only_spec(
                    "readonly",
                    &provider,
                    Arc::new(AtomicUsize::new(0)),
                ))
                .expect("agent");
            assert_eq!(
                agent.config().autonomy.level,
                openhuman_core::security::AutonomyLevel::ReadOnly
            );
            assert!(agent.access().turn_origin().is_none());
            assert!(agent.config().autonomy.trusted_roots.is_empty());
            runtime.remove_agent("readonly").await.unwrap();
            drop(agent);
        })
        .await
        .expect("test task");
    });
}
