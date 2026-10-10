//! A message on a channel bound to an agent (`agent.channel_agents`) runs as
//! that host agent; a binding nobody answers is refused; an unbound channel
//! still goes to the orchestrator over the bus.

use super::*;
use crate::agent::bus::{mock_agent_run_turn, AgentTurnRequest, AgentTurnResponse};
use crate::agent::host_agents::{self, HostAgent, HostAgentResolver};
use crate::channels::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use crate::channels::runtime::process_channel_message;
use crate::channels::{traits, Channel, SendMessage};
use crate::core::runtime::{CoreContext, DomainSet};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tinytools::{PermissionLevel, Tool, ToolResult};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct Telegram {
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Channel for Telegram {
    fn name(&self) -> &str {
        "telegram"
    }
    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(message.content.clone());
        Ok(())
    }
    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn context(channel: Arc<Telegram>, config: Config) -> Arc<ChannelRuntimeContext> {
    let channel: Arc<dyn Channel> = channel;
    Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::from([("telegram".to_string(), channel)])),
        turn_model_source: None,
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("ORCHESTRATOR_PROMPT"),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 5,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        provider_runtime_options: crate::inference::provider::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: Some(Arc::new(config)),
    })
}

fn message(text: &str) -> traits::ChannelMessage {
    traits::ChannelMessage {
        id: "m1".into(),
        sender: "alice".into(),
        sender_name: None,
        reply_target: "42".into(),
        content: text.into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
    }
}

/// An offline config in `tmp`, with `telegram` bound to `agent_id` when given.
fn config(tmp: &tempfile::TempDir, agent_id: Option<&str>) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.local_ai.runtime_enabled = false;
    config.memory.conversations.enabled = false;
    config.agent.session_dual_write = false;
    config.agent.session_shadow_reads = false;
    if let Some(agent_id) = agent_id {
        config
            .agent
            .channel_agents
            .insert("telegram".into(), agent_id.into());
    }
    config
}

#[tokio::test]
async fn a_binding_no_agent_answers_is_refused_not_sent_to_the_orchestrator() {
    let calls = Arc::new(AtomicUsize::new(0));
    let stub_calls = Arc::clone(&calls);
    let _bus = mock_agent_run_turn(move |_req: AgentTurnRequest| {
        stub_calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(AgentTurnResponse::new("orchestrator answered")) }
    })
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let channel = Arc::new(Telegram::default());
    let ctx = context(
        Arc::clone(&channel),
        config(&tmp, Some("nobody-registered")),
    );

    process_channel_message(ctx, message("hello")).await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "never widened to the orchestrator"
    );
    let sent = channel.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("not available"), "{sent:?}");
}

#[tokio::test]
async fn an_unbound_channel_still_runs_the_orchestrator() {
    let calls = Arc::new(AtomicUsize::new(0));
    let stub_calls = Arc::clone(&calls);
    let _bus = mock_agent_run_turn(move |req: AgentTurnRequest| {
        stub_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(req.history[0].content, "ORCHESTRATOR_PROMPT");
        async { Ok(AgentTurnResponse::new("orchestrator answered")) }
    })
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config(&tmp, None);
    config
        .agent
        .channel_agents
        .insert("discord".into(), "some-agent".into());
    let channel = Arc::new(Telegram::default());

    process_channel_message(context(Arc::clone(&channel), config), message("hello")).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        channel.sent.lock().unwrap().clone(),
        vec!["orchestrator answered".to_string()]
    );
}

/// A host tool that records each call it actually ran.
struct Probe {
    name: &'static str,
    level: PermissionLevel,
    runs: Arc<AtomicUsize>,
    origins: Arc<Mutex<Vec<Option<crate::agent::turn_origin::AgentTurnOrigin>>>>,
}

#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "a host probe"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.origins
            .lock()
            .unwrap()
            .push(CoreContext::current_turn_origin());
        Ok(ToolResult::success(format!("{} ran", self.name)))
    }
}

struct OneHostAgent {
    config: Config,
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
    origins: Arc<Mutex<Vec<Option<crate::agent::turn_origin::AgentTurnOrigin>>>>,
}

impl HostAgentResolver for OneHostAgent {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        if agent_id != "teeny-chat" {
            return None;
        }
        let mut definition =
            crate::agent::harness::definition::AgentDefinitionRegistry::builtins_only()
                .get("orchestrator")
                .cloned()
                .unwrap();
        definition.id = agent_id.to_string();
        definition.subagents.clear();
        definition.system_prompt =
            crate::agent::harness::definition::PromptSource::Inline("TEENY_CHAT_PROMPT".into());
        definition.tools = crate::agent::harness::definition::ToolScope::Named(vec![
            "teeny_read".into(),
            "teeny_write".into(),
        ]);
        let (reads, writes) = (Arc::clone(&self.reads), Arc::clone(&self.writes));
        let origins = Arc::clone(&self.origins);
        Some(HostAgent {
            definition,
            config: self.config.clone(),
            host_tools: Some(Arc::new(move |_turn| {
                crate::agent::HostTurnTools::advertised(vec![
                    Box::new(Probe {
                        name: "teeny_read",
                        level: PermissionLevel::ReadOnly,
                        runs: Arc::clone(&reads),
                        origins: Arc::clone(&origins),
                    }),
                    Box::new(Probe {
                        name: "teeny_write",
                        level: PermissionLevel::Write,
                        runs: Arc::clone(&writes),
                        origins: Arc::clone(&origins),
                    }),
                ])
            })),
            hooks: Default::default(),
            context: CoreContext::for_test_with_config(DomainSet::full(), self.config.clone()),
        })
    }
}

/// The tool names a recorded chat request advertised.
fn tool_names(body: &str) -> Vec<String> {
    let body: Value = serde_json::from_str(body).unwrap_or_default();
    body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn tool_call(tool: &str) -> Value {
    json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "fixture",
        "choices": [{ "index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null,
            "tool_calls": [{ "id": format!("call_{tool}"), "type": "function",
                "function": { "name": tool, "arguments": "{}" } }]
        }}],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

fn answer(text: &str) -> Value {
    json!({
        "id": "chatcmpl-2", "object": "chat.completion", "created": 1, "model": "fixture",
        "choices": [{ "index": 0, "finish_reason": "stop",
            "message": { "role": "assistant", "content": text } }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

async fn provider() -> MockServer {
    let server = MockServer::start().await;
    for body in [tool_call("teeny_read"), tool_call("teeny_write")] {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(answer("teeny says hi")))
        .mount(&server)
        .await;
    server
}

/// An agent turn is a deep future; run it the way hosts do, on workers with
/// the documented stack.
fn on_agent_runtime(test: impl std::future::Future<Output = ()> + Send + 'static) {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .build()
        .unwrap()
        .block_on(async { tokio::spawn(test).await.unwrap() });
}

#[test]
fn a_bound_channel_runs_its_host_agent_and_refuses_its_write_tool() {
    // The resolver slot is process-wide; hold it for the whole turn.
    let _slot = host_agents::tests::lock();
    on_agent_runtime(bound_channel_runs_its_host_agent());
}

async fn bound_channel_runs_its_host_agent() {
    let _bus = crate::agent::bus::use_real_agent_handler().await;
    let server = provider().await;
    let tmp = tempfile::tempdir().unwrap();
    // Every host boots the definition registry; a session's hosted authority
    // is built from it.
    crate::agent::harness::AgentDefinitionRegistry::init_global_builtins().unwrap();
    let mut agent_config = config(&tmp, None);
    agent_config.default_model = Some("fixture".into());
    let route = crate::config::schema::EphemeralRoute::from_params(
        Some(format!("{}/v1", server.uri())),
        Some("sk-fixture".into()),
    )
    .unwrap();
    crate::config::schema::ephemeral_route::apply(&mut agent_config, route);
    let (reads, writes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let origins = Arc::new(Mutex::new(Vec::new()));
    let resolver: Arc<dyn HostAgentResolver> = Arc::new(OneHostAgent {
        config: agent_config,
        reads: Arc::clone(&reads),
        writes: Arc::clone(&writes),
        origins: Arc::clone(&origins),
    });
    let channel = Arc::new(Telegram::default());
    let ctx = context(Arc::clone(&channel), config(&tmp, Some("teeny-chat")));

    let outcome = {
        host_agents::install(Arc::clone(&resolver));
        let started = std::time::Instant::now();
        process_channel_message(ctx, message("hello teeny")).await;
        host_agents::clear_if(&resolver);
        started.elapsed()
    };

    assert_eq!(
        channel.sent.lock().unwrap().clone(),
        vec!["teeny says hi".to_string()],
        "the host agent's reply goes back to the chat"
    );
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "the read-only host tool ran"
    );
    assert_eq!(
        writes.load(Ordering::SeqCst),
        0,
        "the write host tool never ran"
    );
    assert!(
        outcome < std::time::Duration::from_secs(60),
        "refused at once, not parked: {outcome:?}"
    );
    let requests = server.received_requests().await.unwrap();
    let chats: Vec<String> = requests
        .iter()
        .filter(|r| r.url.path().ends_with("/chat/completions"))
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect();
    assert_eq!(chats.len(), 3, "{chats:#?}");
    assert!(chats[0].contains("TEENY_CHAT_PROMPT"));
    assert!(!chats[0].contains("ORCHESTRATOR_PROMPT"));
    let advertised = tool_names(&chats[0]);
    assert!(
        advertised.contains(&"teeny_read".to_string()),
        "{advertised:?}"
    );
    assert!(
        !advertised.contains(&"teeny_write".to_string()),
        "a tool above the channel's ceiling is not offered: {advertised:?}"
    );
    assert!(
        chats[2].contains("teeny_write") && !chats[2].contains("teeny_write ran"),
        "the write call came back as a refusal"
    );
    let origins = origins.lock().unwrap().clone();
    assert!(
        matches!(
            origins.as_slice(),
            [Some(crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel { channel, .. })]
                if channel == "telegram"
        ),
        "the read-only tool ran under the channel's origin: {origins:?}"
    );
}
