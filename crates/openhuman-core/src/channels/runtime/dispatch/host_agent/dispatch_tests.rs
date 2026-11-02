//! A message on a channel bound to an agent (`agent.channel_agents`) runs as
//! that host agent; a binding nobody answers is refused; an unbound channel
//! still reaches the orchestrator.

use super::*;
use crate::agent::host_agents::{self, HostAgent, HostAgentResolver};
use crate::channels::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use crate::channels::runtime::process_channel_message;
use crate::core::runtime::{CoreContext, DomainSet};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tinychannels_bus as traits;
use tinytools::{PermissionLevel, Tool, ToolResult};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[path = "dispatch_unbound_tests.rs"]
mod unbound;

fn context(
    channel: Arc<crate::channels::runtime::TestChannel>,
    config: Config,
) -> Arc<ChannelRuntimeContext> {
    Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(HashMap::from([(
            "telegram".to_string(),
            Arc::new(crate::channels::runtime::ChannelHandle::Test(channel)),
        )])),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: None,
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("ORCHESTRATOR_PROMPT"),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 5,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: Some(Arc::new(config)),
        core: None,
    })
}

fn message(text: &str) -> traits::ChannelMessage {
    traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
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
    config.modules.source = crate::config::schema::ModuleSourceMode::Linked;
    config.modules.allow_download = false;
    config.local_ai.runtime_enabled = false;
    config.memory.conversations.enabled = false;
    config.agent.session_dual_write = false;
    config.agent.session_shadow_reads = false;
    crate::security::credentials::api_key::store_api_key(&config, "test-session-fixture-key")
        .expect("store channel test credential");
    if let Some(agent_id) = agent_id {
        config
            .agent
            .channel_agents
            .insert("telegram".into(), agent_id.into());
    }
    config
}

/// A host tool that records each call it actually ran.
struct Probe {
    name: &'static str,
    level: PermissionLevel,
    runs: Arc<AtomicUsize>,
    origins: Arc<Mutex<Vec<Option<crate::security::turn_origin::AgentTurnOrigin>>>>,
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
    origins: Arc<Mutex<Vec<Option<crate::security::turn_origin::AgentTurnOrigin>>>>,
}

impl HostAgentResolver for OneHostAgent {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        if agent_id != "teeny-chat" {
            return None;
        }
        let mut definition =
            crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
                .get("orchestrator")
                .cloned()
                .unwrap();
        definition.id = agent_id.to_string();
        definition.subagents.clear();
        definition.system_prompt =
            tinyagents_definition::PromptSource::Inline("TEENY_CHAT_PROMPT".into());
        definition.tools = tinyagents_definition::ToolScope::Named(vec![
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

async fn delayed_provider(
    delay: std::time::Duration,
    request_arrived: Arc<tokio::sync::Notify>,
    request_path: Arc<Mutex<Option<String>>>,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(move |request: &wiremock::Request| {
            *request_path.lock().expect("request path lock") = Some(request.url.path().to_string());
            request_arrived.notify_one();
            ResponseTemplate::new(200)
                .set_body_json(answer("this response arrives after the channel deadline"))
                .set_delay(delay)
        })
        .mount(&server)
        .await;
    server
}

struct InstalledResolver(Arc<dyn HostAgentResolver>);

impl InstalledResolver {
    fn new(resolver: Arc<dyn HostAgentResolver>) -> Self {
        host_agents::install(Arc::clone(&resolver));
        Self(resolver)
    }
}

impl Drop for InstalledResolver {
    fn drop(&mut self) {
        host_agents::clear_if(&self.0);
    }
}

/// An agent turn is a deep future; run it the way hosts do, on a worker with
/// the documented stack. The future is built on that worker, so it may hold
/// non-`Send` guards (the storage-slot lock) across awaits.
fn on_agent_runtime<F, Fut>(make: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
                .build()
                .unwrap()
                .block_on(make())
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_bound_channel_runs_its_host_agent_and_refuses_its_write_tool() {
    // The resolver slot is process-wide; hold it for the whole turn.
    let _slot = host_agents::tests::lock();
    on_agent_runtime(bound_channel_runs_its_host_agent);
}

async fn bound_channel_runs_its_host_agent() {
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;
    let _bus = crate::agent::bus::use_real_agent_handler().await;
    let server = provider().await;
    let tmp = tempfile::tempdir().unwrap();
    // Every host boots the definition registry; a session's hosted authority
    // is built from it.
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .unwrap();
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
    let channel = Arc::new(crate::channels::runtime::TestChannel::new("telegram"));
    let ctx = context(Arc::clone(&channel), config(&tmp, Some("teeny-chat")));

    let outcome = {
        host_agents::install(Arc::clone(&resolver));
        let started = std::time::Instant::now();
        process_channel_message(ctx, message("hello teeny")).await;
        host_agents::clear_if(&resolver);
        started.elapsed()
    };

    assert_eq!(
        channel
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
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
            [Some(crate::security::turn_origin::AgentTurnOrigin::ExternalChannel { channel, .. })]
                if channel == "telegram"
        ),
        "the read-only tool ran under the channel's origin: {origins:?}"
    );
}

#[test]
fn native_host_agent_turn_enforces_the_configured_channel_deadline() {
    std::thread::Builder::new()
        .name("channel-deadline-test".into())
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            let _slot = host_agents::tests::lock();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(native_host_agent_turn_enforces_the_configured_channel_deadline_inner());
        })
        .unwrap()
        .join()
        .expect("channel deadline runtime thread");
}

async fn native_host_agent_turn_enforces_the_configured_channel_deadline_inner() {
    let env_lock = crate::config::test_env::EnvVarGuard::locked_async().await;
    let provider_server = MockServer::start().await;
    let _provider_base = env_lock.with("OPENHUMAN_TELEGRAM_BOT_API_BASE", provider_server.uri());
    Mock::given(method("POST"))
        .and(path("/bottoken/getUpdates"))
        .respond_with({
            let poll_count = Arc::new(AtomicUsize::new(0));
            move |_request: &wiremock::Request| {
                let updates = if poll_count.fetch_add(1, Ordering::SeqCst) == 0 {
                    serde_json::json!([{
                        "update_id": 1,
                        "message": {
                            "message_id": 11,
                            "date": 1,
                            "chat": { "id": 42, "type": "private" },
                            "from": { "id": 42, "is_bot": false, "first_name": "Alice" },
                            "text": "slow native turn"
                        }
                    }])
                } else {
                    serde_json::json!([])
                };
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "result": updates
                }))
            }
        })
        .mount(&provider_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/deleteWebhook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": true
        })))
        .mount(&provider_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/bottoken/getMe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": { "id": 7, "is_bot": true, "username": "fixture_bot" }
        })))
        .mount(&provider_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 991,
                "date": 1,
                "chat": { "id": 42, "type": "private" },
                "text": "reply"
            }
        })))
        .mount(&provider_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/editMessageText"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 991,
                "date": 1,
                "chat": { "id": 42, "type": "private" },
                "text": "working"
            }
        })))
        .mount(&provider_server)
        .await;
    let _bus = crate::agent::bus::use_real_agent_handler().await;
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;
    let timeout = tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS;
    let request_arrived = Arc::new(tokio::sync::Notify::new());
    let request_path = Arc::new(Mutex::new(None));
    let server = delayed_provider(
        std::time::Duration::from_secs(timeout + 1),
        Arc::clone(&request_arrived),
        Arc::clone(&request_path),
    )
    .await;
    let tmp = tempfile::tempdir().unwrap();
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .unwrap();
    let mut agent_config = config(&tmp, None);
    agent_config.default_model = Some("fixture".into());
    let route = crate::config::schema::EphemeralRoute::from_params(
        Some(format!("{}/v1", server.uri())),
        Some("sk-fixture".into()),
    )
    .unwrap();
    crate::config::schema::ephemeral_route::apply(&mut agent_config, route);
    let resolver: Arc<dyn HostAgentResolver> = Arc::new(OneHostAgent {
        config: agent_config,
        reads: Arc::new(AtomicUsize::new(0)),
        writes: Arc::new(AtomicUsize::new(0)),
        origins: Arc::new(Mutex::new(Vec::new())),
    });
    let mut channel_config = config(&tmp, Some("teeny-chat"));
    channel_config.modules.source = crate::config::schema::ModuleSourceMode::Linked;
    channel_config.modules.allow_download = false;
    channel_config.channels_config.telegram = Some(tinychannels_bus::config::TelegramConfig {
        bot_token: "token".into(),
        chat_id: None,
        allowed_users: vec!["42".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1,
        silent_streaming: true,
        mention_only: false,
    });
    channel_config.channel_voice.reply_mode = crate::config::schema::VoiceReplyMode::Never;
    let (dispatch_tx, mut dispatch_rx) = tokio::sync::mpsc::channel(4);
    let context_slot = Arc::new(std::sync::OnceLock::new());
    let mut module_runtime = crate::channels::runtime::prepare_module_runtime_for_test_in_context(
        Arc::new(channel_config.clone()),
        Arc::clone(&context_slot),
        dispatch_tx,
    )
    .await
    .expect("the configured linked Telegram module prepares");
    let configured_channels = module_runtime.channels();
    let configured_module_objects = module_runtime.module_objects();
    let (configured_key, configured_channel) = configured_channels
        .iter()
        .next()
        .expect("the Telegram config resolves one configured channel handle");
    let configured_key = configured_key.clone();
    let configured_channel_name = configured_channel.name().to_owned();
    assert!(configured_module_objects.contains_key(&configured_key));

    let channel = Arc::new(crate::channels::runtime::TestChannel::new("telegram"));
    let mut ctx = context(Arc::clone(&channel), channel_config.clone());
    Arc::get_mut(&mut ctx)
        .expect("the test owns the runtime context")
        .message_timeout_secs = timeout;
    Arc::get_mut(&mut ctx)
        .expect("the test still owns the runtime context")
        .channels_by_name = configured_channels;
    Arc::get_mut(&mut ctx)
        .expect("the test still owns the runtime context")
        .module_objects = configured_module_objects;
    context_slot
        .set(Arc::downgrade(&ctx))
        .expect("module callbacks receive the live host context");
    module_runtime
        .start(&channel_config)
        .await
        .expect("the configured Telegram provider starts against the mock API");

    let inbound = tokio::time::timeout(std::time::Duration::from_secs(10), dispatch_rx.recv())
        .await
        .expect("the provider delivers its mocked update")
        .expect("the provider dispatch channel remains open");
    let account_id = inbound
        .inbound_envelope
        .as_ref()
        .and_then(|envelope| envelope.channel.account_id.as_deref());
    assert_eq!(account_id, Some(configured_key.as_str()));
    assert_eq!(
        inbound
            .inbound_envelope
            .as_ref()
            .map(|envelope| envelope.channel.id.as_str()),
        Some(configured_channel_name.as_str())
    );

    let _installed = InstalledResolver::new(resolver);
    let mut turn = tokio::spawn(
        crate::channels::runtime::dispatch::process_channel_runtime_message(ctx, inbound),
    );
    tokio::select! {
        result = &mut turn => panic!(
            "channel pipeline completed before a model request: result={result:?}, provider_requests={:#?}",
            server.received_requests().await.unwrap(),
        ),
        arrived = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            request_arrived.notified(),
        ) => if arrived.is_err() {
            panic!(
                "no model request reached the delayed provider; provider_requests={:#?}",
                server.received_requests().await.unwrap(),
            );
        },
    }
    assert!(
        request_path
            .lock()
            .expect("request path lock")
            .as_deref()
            .is_some_and(|path| path.ends_with("/chat/completions")),
        "the delayed request is a model completion: {request_path:?}"
    );
    // The linked module and mock inference server perform I/O on their own
    // runtimes. Use real time here: pausing this test runtime does not pause
    // those requests and can fire the unrelated web backstop first.
    let turn_result = tokio::time::timeout(std::time::Duration::from_secs(timeout + 15), &mut turn)
        .await
        .expect("native channel turn completes within the real-time test bound");
    turn_result.expect("native channel turn task completes");
    let provider_requests = provider_server
        .received_requests()
        .await
        .expect("mock Telegram requests");
    let provider_observations = provider_requests
        .iter()
        .map(|request| {
            (
                request.url.path().to_owned(),
                serde_json::from_slice::<serde_json::Value>(&request.body).ok(),
            )
        })
        .collect::<Vec<_>>();
    println!("fixture Telegram provider requests: {provider_observations:#?}");
    let timeout_notice = "past its time budget";
    let late_model_reply = "this response arrives after the channel deadline";
    let terminal_sends = provider_observations
        .iter()
        .filter(|(path, body)| {
            matches!(
                path.as_str(),
                "/bottoken/sendMessage" | "/bottoken/editMessageText"
            ) && body
                .as_ref()
                .and_then(|body| body["text"].as_str())
                .is_some_and(|text| {
                    text.contains(timeout_notice) || text.contains(late_model_reply)
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        terminal_sends.len(),
        1,
        "the turn emits one terminal timeout or late-model message; progressive placeholders are allowed: {provider_observations:#?}"
    );
    let terminal_text = terminal_sends[0]
        .1
        .as_ref()
        .and_then(|body| body["text"].as_str())
        .expect("terminal Telegram output has text");
    assert!(
        terminal_text.contains(timeout_notice),
        "the actual module provider receives the classified timeout notice; observed terminal text: {terminal_text:?}"
    );

    // Let the delayed model response itself finish before checking that the
    // cancelled turn cannot publish a second terminal provider request.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let late_model_deliveries = provider_server
        .received_requests()
        .await
        .expect("mock Telegram requests")
        .iter()
        .filter(|request| {
            matches!(
                request.url.path(),
                "/bottoken/sendMessage" | "/bottoken/editMessageText"
            ) && serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| body["text"].as_str().map(str::to_owned))
                .is_some_and(|text| text.contains(late_model_reply))
        })
        .count();
    assert_eq!(
        late_model_deliveries, 0,
        "the delayed model response cannot deliver after timeout cleanup"
    );
    module_runtime
        .close()
        .await
        .expect("the configured Telegram module shuts down");
}
