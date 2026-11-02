//! Debug-build harnesses for raw integration coverage of the channel runtime.

#[allow(unused_imports)] // Used by tests/raw_coverage outside the core unit-test crate.
pub use super::dispatch::test_support::build_channel_context_block_for_test;
use super::dispatch::{
    process_channel_message, process_channel_runtime_message, RuntimeChannelMessage,
};
use crate::channels::context::ChannelRuntimeContext;
use crate::channels::ChannelSystemPrompt;
use crate::config::{MultimodalConfig, MultimodalFileConfig, ReliabilityConfig};
use crate::core::events::DomainEvent;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tinychannels_bus::module_runtime::CHANNEL_MESSAGE_TIMEOUT_SECS;
use tinychannels_bus::ChannelMessage;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};
use tinytools::{Tool, ToolResult};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Debug, Clone)]
pub struct DispatchHarnessOptions {
    pub channel_name: String,
    pub content: String,
    pub thread_ts: Option<String>,
    pub inbound_envelope: Option<tinychannels_bus::ChannelInboundEnvelope>,
    pub supports_reactions: bool,
    pub response_text: Option<String>,
    pub handler_error: Option<String>,
    pub handler_delay_ms: u64,
    pub timeout_secs: u64,
    /// Workspace the runtime context reports; `None` uses an isolated temporary workspace.
    ///
    /// `pub`, like every other field here, and it has to be: integration tests
    /// under `tests/` are a separate crate, and a struct-update expression
    /// (`..DispatchHarnessOptions::default()`) requires **every** field to be
    /// visible — including the ones the caller never names. Added as
    /// `pub(crate)` in `a45b1c1af`, which broke three of them with E0451.
    pub workspace_dir: Option<PathBuf>,
    /// The prompt every dispatch is seeded with; the default pins a fixed literal.
    /// Pass one `ChannelSystemPrompt::refreshing` clone to several dispatches
    /// to observe a re-render across them.
    ///
    /// Wrapped in [`HarnessSystemPrompt`] rather than exposing
    /// `Option<ChannelSystemPrompt>` — see that type for why.
    pub system_prompt: HarnessSystemPrompt,
}

/// Opaque carrier for the harness's system prompt.
///
/// [`DispatchHarnessOptions`] is `pub` and integration tests build it with
/// `..DispatchHarnessOptions::default()`, so every field must be visible from
/// another crate. The prompt itself must **not** become visible with it:
/// `ChannelSystemPrompt` is `pub(crate)`, its `Refreshing` variant wraps a
/// `Mutex`-backed render cache (`RefreshingInner`), and widening it would
/// cascade to `RefreshingInner` and `ChannelPromptInputs` — production types
/// with real invariants — purely to satisfy a test harness. A `pub` field of a
/// `pub(crate)` type is also exactly what the `private_interfaces` lint exists
/// to catch, and `-D warnings` makes that a hard error.
///
/// A `pub` newtype with a **private** field settles both: the field is visible,
/// the type inside it is not. An out-of-crate caller can therefore only ever
/// get the default (no prompt), which is all those tests want; in-crate tests
/// set one with [`HarnessSystemPrompt::new`].
#[derive(Clone, Default)]
pub struct HarnessSystemPrompt(Option<ChannelSystemPrompt>);

impl HarnessSystemPrompt {
    /// Seed the harness with `prompt`. Crate-internal, because the prompt is.
    pub(crate) fn new(prompt: ChannelSystemPrompt) -> Self {
        Self(Some(prompt))
    }

    /// The prompt, if one was set.
    pub(crate) fn into_inner(self) -> Option<ChannelSystemPrompt> {
        self.0
    }
}

impl std::fmt::Debug for HarnessSystemPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(prompt) => f.debug_tuple("HarnessSystemPrompt").field(prompt).finish(),
            None => f.write_str("HarnessSystemPrompt(default)"),
        }
    }
}

impl Default for DispatchHarnessOptions {
    fn default() -> Self {
        Self {
            channel_name: "test-channel".to_string(),
            content: "hello".to_string(),
            thread_ts: None,
            inbound_envelope: None,
            supports_reactions: false,
            response_text: Some("dispatch ok".to_string()),
            handler_error: None,
            handler_delay_ms: 0,
            timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            workspace_dir: None,
            system_prompt: HarnessSystemPrompt::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSend {
    pub kind: &'static str,
    pub recipient: String,
    pub content: String,
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DispatchHarnessObservation {
    pub sends: Vec<ObservedSend>,
    pub start_typing_calls: usize,
    pub stop_typing_calls: usize,
    pub received_event_envelope: Option<tinychannels_bus::ChannelInboundEnvelope>,
    /// Requests observed at the real `ChatModel` boundary.
    pub model_calls: Vec<HarnessModelCall>,
}

/// A real linked TinyChannels runtime backed by a loopback provider endpoint.
///
/// Tests that need to exercise host policy together with channel operations
/// should use this fixture rather than a synthetic `Channel` implementation.
pub(crate) struct LinkedTelegramRuntime {
    _env: crate::config::test_env::EnvVarGuard,
    _storage: crate::channels::module_fixture::MemoryStorageGuard,
    _workspace: tempfile::TempDir,
    server: MockServer,
    config: crate::config::Config,
    runtime: crate::channels::runtime::ModuleRuntimeTestGuard,
}

impl LinkedTelegramRuntime {
    pub(crate) fn channels(
        &self,
    ) -> Arc<HashMap<String, Arc<crate::channels::runtime::ChannelHandle>>> {
        self.runtime.channels()
    }

    pub(crate) fn module_objects(
        &self,
    ) -> Arc<HashMap<String, tinybus::module::ModuleObjectClient>> {
        self.runtime.module_objects()
    }

    pub(crate) fn config(&self) -> &crate::config::Config {
        &self.config
    }

    pub(crate) fn provider_endpoint(&self) -> String {
        format!("{}/v1", self.server.uri())
    }

    pub(crate) async fn provider_request_paths(&self) -> Vec<String> {
        self.server
            .received_requests()
            .await
            .expect("mock provider should expose received requests")
            .into_iter()
            .map(|request| request.url.path().to_owned())
            .collect()
    }

    pub(crate) async fn provider_request_bodies(&self, path: &str) -> Vec<serde_json::Value> {
        self.server
            .received_requests()
            .await
            .expect("mock provider should expose received requests")
            .into_iter()
            .filter(|request| request.url.path() == path)
            .map(|request| {
                serde_json::from_slice(&request.body).expect("provider request should be JSON")
            })
            .collect()
    }
}

/// Prepare an actual linked TinyChannels module with mocked Telegram HTTP.
pub(crate) async fn linked_telegram_runtime() -> LinkedTelegramRuntime {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottoken/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/deleteWebhook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/sendChatAction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/setMessageReaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 991,
                "date": 1,
                "chat": { "id": 42, "type": "private" },
                "text": "dispatch ok"
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"typing fixture reply\"}}]}\n\n",
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            )
            .as_bytes()
            .to_vec(),
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let env = crate::config::test_env::EnvVarGuard::locked_async()
        .await
        .with("OPENHUMAN_TELEGRAM_BOT_API_BASE", server.uri());
    let storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;

    let workspace = tempfile::tempdir().expect("temporary module workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.action_dir = workspace.path().to_path_buf();
    config.modules.allow_download = false;
    config.modules.source = crate::config::schema::ModuleSourceMode::Linked;
    config.channels_config.telegram = Some(tinychannels_bus::config::TelegramConfig {
        bot_token: "token".into(),
        chat_id: None,
        allowed_users: vec!["42".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1,
        silent_streaming: true,
        mention_only: false,
    });
    let (dispatch, _receiver) = tokio::sync::mpsc::channel(4);
    let mut runtime = crate::channels::runtime::prepare_module_runtime_for_test(
        Arc::new(config.clone()),
        dispatch,
    )
    .await
    .expect("linked TinyChannels runtime should prepare");
    runtime
        .start(&config)
        .await
        .expect("mock Telegram provider should start without network access");

    LinkedTelegramRuntime {
        _env: env,
        _storage: storage,
        _workspace: workspace,
        server,
        config,
        runtime,
    }
}

/// Bind a test runtime context to a real core context so module callbacks that
/// clear durable channel history exercise the host reset path.
pub(crate) fn bind_test_core_context(
    context: &mut crate::channels::context::ChannelRuntimeContext,
    base_config: &crate::config::Config,
) -> Arc<crate::core::runtime::CoreContext> {
    bind_test_core_context_with_provider_endpoint(context, base_config, "http://127.0.0.1:9/v1")
}

pub(crate) fn bind_test_core_context_with_provider_endpoint(
    context: &mut crate::channels::context::ChannelRuntimeContext,
    base_config: &crate::config::Config,
    provider_endpoint: &str,
) -> Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    let workspace = context.workspace_dir.as_ref().clone();
    let mut config = base_config.clone();
    config.workspace_dir = workspace.clone();
    config.action_dir = workspace.clone();
    config.config_path = workspace.join("config.toml");
    config.cloud_providers = ["openai", "test-provider"]
        .into_iter()
        .map(|slug| crate::config::schema::CloudProviderCreds {
            id: format!("p_{slug}_channel_test"),
            slug: slug.into(),
            label: "Channel test provider".into(),
            endpoint: provider_endpoint.into(),
            default_model: Some("channel-test-model".into()),
            ..Default::default()
        })
        .collect();
    context.config = Some(Arc::new(config.clone()));
    let config_for_credential = config.clone();
    let core = CoreContext::for_test(DomainSet::none(), Some(workspace)).derive_with(
        ContextOverlay::new(config, DomainSet::full(), Default::default())
            .profile("channel-handle-test"),
    );
    context.core = Some(Arc::downgrade(&core));
    // A standing API-key credential satisfies the backend session gate that
    // building the host agent's model checks, without a signed-in user. Store
    // it inside the context so a storage-backed secret store (installed by the
    // module fixture) records it under the scope the turn later reads.
    let stored_config = config_for_credential;
    CoreContext::sync_scope(Arc::clone(&core), || {
        crate::security::credentials::api_key::store_api_key(
            &stored_config,
            "test-session-fixture-key",
        )
    })
    .expect("store channel test credential");
    core
}

#[derive(Debug, Clone)]
pub struct HarnessModelCall {
    pub roles: Vec<String>,
    pub text: String,
    pub model: Option<String>,
    pub requested_route: Option<String>,
}

struct HarnessModel {
    response_text: String,
    error: Option<String>,
    delay: Duration,
    calls: Arc<Mutex<Vec<HarnessModelCall>>>,
}

#[async_trait]
impl ChatModel<()> for HarnessModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(HarnessModelCall {
                roles: request
                    .messages
                    .iter()
                    .map(|message| match message {
                        tinyinference_llm::message::Message::System(_) => "system",
                        tinyinference_llm::message::Message::User(_) => "user",
                        tinyinference_llm::message::Message::Assistant(_) => "assistant",
                        tinyinference_llm::message::Message::Tool(_) => "tool",
                        tinyinference_llm::message::Message::Custom(_) => "custom",
                    })
                    .map(str::to_owned)
                    .collect(),
                text: request
                    .messages
                    .iter()
                    .map(|message| message.text())
                    .collect::<Vec<_>>()
                    .join("\n---\n"),
                model: request.model,
                requested_route: request.requested_route,
            });
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        if let Some(error) = &self.error {
            return Err(tinyinference_llm::Error::Model(error.clone()));
        }
        Ok(ModelResponse::assistant(self.response_text.clone()))
    }
}

struct HarnessTool;

#[async_trait]
impl Tool for HarnessTool {
    fn name(&self) -> &str {
        "harness_tool"
    }

    fn description(&self) -> &str {
        "debug harness tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

pub async fn run_dispatch_harness(options: DispatchHarnessOptions) -> DispatchHarnessObservation {
    // The real agent handler and global registries are process-wide. Serialize
    // the whole harness run so concurrent tests cannot replace them mid-turn.
    let _agent_handler_guard = crate::agent::bus::use_real_agent_handler().await;
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;

    crate::core::bus::init().await.expect("bus init");
    let mut event_rx = crate::core::bus::BUS
        .get()
        .expect("bus initialised")
        .receiver();
    let _ = crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins();

    let response_text = options
        .response_text
        .clone()
        .unwrap_or_else(|| "dispatch ok".to_string());
    let model_calls = Arc::new(Mutex::new(Vec::new()));

    let mut state = crate::channels::runtime::TestChannel::new(options.channel_name.clone());
    state.supports_reactions = options.supports_reactions;
    let state = Arc::new(state);
    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(
        options.channel_name.clone(),
        Arc::new(crate::channels::runtime::ChannelHandle::Test(Arc::clone(
            &state,
        ))),
    );

    let model: Arc<dyn ChatModel<()>> = Arc::new(HarnessModel {
        response_text,
        error: options.handler_error.clone(),
        delay: Duration::from_millis(options.handler_delay_ms),
        calls: Arc::clone(&model_calls),
    });
    let mut provider_cache = HashMap::new();
    provider_cache.insert(
        "harness-provider".to_string(),
        crate::agent::tinyagents::TurnModelSource::from_model(Arc::clone(&model)),
    );
    let harness_system_prompt = options
        .system_prompt
        .clone()
        .into_inner()
        .unwrap_or_else(|| ChannelSystemPrompt::fixed("system prompt"));
    let (harness_workspace_dir, _workspace_guard) = match options.workspace_dir.clone() {
        Some(workspace) => (workspace, None),
        None => {
            let workspace = tempfile::tempdir().expect("temporary dispatch harness workspace");
            (workspace.path().to_path_buf(), Some(workspace))
        }
    };
    let ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(model)),
        default_provider: Arc::new("harness-provider".to_string()),
        tools_registry: Arc::new(vec![Box::new(HarnessTool) as Box<dyn Tool>]),
        system_prompt: harness_system_prompt,
        model: Arc::new("harness-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 3,
        turn_model_source_cache: Arc::new(Mutex::new(provider_cache)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(ReliabilityConfig::default()),
        workspace_dir: Arc::new(harness_workspace_dir),
        // Mirror the owning TinyChannels runtime's validated floor even when
        // a test supplies an undersized value directly.
        message_timeout_secs: options
            .timeout_secs
            .max(tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS),
        multimodal: MultimodalConfig::default(),
        multimodal_files: MultimodalFileConfig::default(),
        config: None,
        core: None,
    });

    let expected_event_channel = options.channel_name.clone();

    let message = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "m1".to_string(),
        sender: "alice".to_string(),
        reply_target: "reply".to_string(),
        content: options.content,
        channel: options.channel_name,
        timestamp: 1,
        thread_ts: options.thread_ts,
        sender_name: None,
    };
    if let Some(inbound_envelope) = options.inbound_envelope {
        process_channel_runtime_message(
            Arc::clone(&ctx),
            RuntimeChannelMessage::with_inbound_envelope(message, inbound_envelope),
        )
        .await;
    } else {
        process_channel_message(Arc::clone(&ctx), message).await;
    }

    let received_event_envelope = loop {
        match event_rx.try_recv() {
            Ok(DomainEvent::ChannelMessageReceived {
                channel,
                message_id,
                inbound_envelope,
                ..
            }) if channel == expected_event_channel && message_id == "m1" => {
                break inbound_envelope;
            }
            Ok(_) => {}
            Err(_) => break None,
        }
    };

    let sends = state
        .sent
        .lock()
        .expect("test sends lock")
        .iter()
        .map(|message| ObservedSend {
            kind: "send",
            recipient: message.recipient.clone(),
            content: message.content.clone(),
            thread_ts: message.thread_ts.clone(),
        })
        .collect();
    let model_calls = model_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    DispatchHarnessObservation {
        sends,
        start_typing_calls: state.start_typing_calls.load(Ordering::SeqCst),
        stop_typing_calls: state.stop_typing_calls.load(Ordering::SeqCst),
        received_event_envelope,
        model_calls,
    }
}
