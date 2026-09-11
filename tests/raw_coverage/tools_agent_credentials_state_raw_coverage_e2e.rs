//! Round16 raw integration coverage for tools, agent delegation, credentials, app state, and config.
//!
//! These tests stay on loopback services and temp workspaces. They exercise
//! public Rust surfaces only, so they cover the same paths used by the core RPC
//! and agent runtime without launching a real browser, hitting the network, or
//! touching the OS keychain.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::harness::session::Agent;
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, AgentDefinition, ParentExecutionContext, PromptSource,
    SandboxMode, SubagentRunOptions, ToolScope,
};
use openhuman_core::openhuman::desktop::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::config::{
    BrowserConfig, Config, HttpRequestConfig, McpAuthConfig, McpServerConfig,
};
use openhuman_core::openhuman::agent::context::prompt::ToolCallFormat;
use openhuman_core::openhuman::security::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary};
use openhuman_core::openhuman::security::{AuditLogger, SecurityPolicy};
use openhuman_core::openhuman::inference::tokenjuice::AgentTokenjuiceCompression;
use openhuman_core::openhuman::tools::{
    all_tools, BrowserTool, ComputerUseConfig, SpawnSubagentTool, Tool, ToolResult,
};
use parking_lot::Mutex as ParkingMutex;
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};
use tinyinference::message::{AssistantMessage, ContentBlock, Message};
use tinyinference::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyinference::tool::ToolCall;
use tinyinference::usage::Usage;

static ROUND16_ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct Harness {
    _tmp: TempDir,
    root: PathBuf,
    workspace: PathBuf,
    _guards: Vec<EnvGuard>,
}

impl Harness {
    async fn config(&self) -> Config {
        config_rpc::load_config_with_timeout()
            .await
            .expect("isolated config should load")
    }

    fn app_state_file(&self) -> PathBuf {
        self.workspace.join("state/app-state.json")
    }
}

struct ScriptedModel {
    responses: ParkingMutex<Vec<ModelResponse>>,
    requests: ParkingMutex<Vec<Vec<Message>>>,
}

impl ScriptedModel {
    fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            responses: ParkingMutex::new(responses),
            requests: ParkingMutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Vec<Message>> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: OnceLock<ModelProfile> = OnceLock::new();
        Some(PROFILE.get_or_init(|| ModelProfile {
            provider: Some("round16".to_string()),
            tool_calling: true,
            parallel_tool_calls: true,
            ..ModelProfile::default()
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.requests.lock().push(request.messages);
        Ok(self.responses.lock().remove(0))
    }
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: openhuman_core::openhuman::memory::RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "round16-memory"
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo deterministic test content"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "message": { "type": "string" } }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        Ok(ToolResult::success(format!(
            "echo:{}",
            args.get("message")
                .and_then(Value::as_str)
                .unwrap_or("missing")
        )))
    }
}

#[derive(Clone, Default)]
struct SidecarState {
    requests: Arc<Mutex<Vec<Value>>>,
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ROUND16_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("create target");
    Builder::new()
        .prefix("tools-agent-credentials-state-round16-")
        .tempdir_in("target")
        .expect("round16 tempdir")
}

fn write_min_config(root: &Path, api_url: &str) {
    std::fs::create_dir_all(root).expect("create openhuman root");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "round16-coverage-model"
default_temperature = 0.0
onboarding_completed = true
chat_onboarding_completed = true

[observability]
analytics_enabled = false

[secrets]
encrypt = false

[meet]
auto_orchestrator_handoff = true

[local_ai]
enabled = false
runtime_enabled = false
opt_in_confirmed = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0
auto_save = false

[memory_tree]
embedding_strict = false
"#
    );
    std::fs::write(root.join("config.toml"), &cfg).expect("write config.toml");
    let _: Config = toml::from_str(&cfg).expect("round16 config must match schema");
}

fn setup(api_url: &str) -> Harness {
    let tmp = tempdir();
    let root = tmp.path().join("openhuman");
    write_min_config(&root, api_url);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let guards = vec![
        EnvGuard::set_to_path("OPENHUMAN_WORKSPACE", &root),
        EnvGuard::set_to_path("HOME", tmp.path()),
        EnvGuard::unset("BACKEND_URL"),
        EnvGuard::unset("VITE_BACKEND_URL"),
        EnvGuard::unset("OPENHUMAN_API_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvGuard::unset("OPENHUMAN_CORE_PORT"),
        EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
        EnvGuard::unset("OPENHUMAN_BROWSER_ALLOW_ALL"),
        EnvGuard::unset("OPENHUMAN_LSP_ENABLED"),
    ];

    Harness {
        _tmp: tmp,
        root,
        workspace,
        _guards: guards,
    }
}

fn usage(input_tokens: u64, output_tokens: u64) -> Usage {
    let mut usage = Usage::new(input_tokens, output_tokens);
    usage.cache_read_tokens = input_tokens / 2;
    usage
}

fn response(text: Option<&str>, tool_calls: Vec<ToolCall>) -> ModelResponse {
    let usage = usage(50, 7);
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: text
                .map(|text| vec![ContentBlock::Text(text.to_string())])
                .unwrap_or_default(),
            tool_calls,
            usage: Some(usage),
        },
        usage: Some(usage),
        finish_reason: None,
        raw: None,
        resolved_model: None,
        continue_turn: None,
            served_from_cache: false,
    }
}

fn parent_context(workspace: PathBuf, provider: Arc<ScriptedModel>) -> ParentExecutionContext {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let tool_specs = tools.iter().map(|tool| tool.spec()).collect();
    ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: [
            "test".to_string(),
            "tools_agent".to_string(),
            "integrations_agent".to_string(),
        ]
        .into_iter()
        .collect(),
        turn_model_source: openhuman_core::openhuman::agent::tinyagents::TurnModelSource::from_model(
            provider,
        ),
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(tool_specs),
        // #6145: empty means "same surface as `all_tool_specs`" — the
        // catalogue falls back to it, so these stubs keep the behaviour
        // they had before the parent's visible set became its own field.
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "round16-model".to_string(),
        temperature: 0.0,
        workspace_dir: workspace,
        workspace_descriptor: None,
        memory: Arc::new(StubMemory),
        agent_config: openhuman_core::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            ..Default::default()
        },
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("parent memory".to_string())),
        session_id: "round16-parent".to_string(),
        channel: "round16-channel".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "1710000000_parent".to_string(),
        session_parent_prefix: Some("root".to_string()),
        on_progress: None,
        run_queue: None,
    }
}

fn agent_definition(id: &str, max_result_chars: Option<usize>) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        when_to_use: "Raw coverage test agent".to_string(),
        display_name: Some("Round16 Agent".to_string()),
        system_prompt: PromptSource::Inline("Use the visible tools and answer tersely.".into()),
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: true,
        omit_profile: true,
        omit_memory_md: true,
        model: Default::default(),
        temperature: 0.0,
        tools: ToolScope::Named(vec!["echo".to_string()]),
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 2,
        iteration_policy: Default::default(),
        max_result_chars,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::ReadOnly,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression: AgentTokenjuiceCompression::Auto,
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: Default::default(),
        source: Default::default(),
        graph: Default::default(),
    }
}

fn tool_names(tools: &[Box<dyn Tool>]) -> Vec<String> {
    let mut names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn start_computer_sidecar(state: SidecarState) -> String {
    async fn handler(
        State(state): State<SidecarState>,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        state.requests.lock().expect("requests").push(body.clone());
        Json(json!({
            "success": true,
            "data": {
                "backend": "computer_use",
                "echo_action": body["action"].clone(),
                "x": body["params"]["x"].clone()
            }
        }))
    }

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind computer sidecar");
    let addr = listener.local_addr().expect("sidecar addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", post(handler)).with_state(state),
        )
        .await
        .expect("serve computer sidecar");
    });
    format!("http://{addr}/")
}

fn browser_tool(endpoint: String, workspace: &Path) -> BrowserTool {
    let security = Arc::new(SecurityPolicy::from_config(
        &Config::default().autonomy,
        workspace,
        workspace,
    ));
    BrowserTool::new_with_backend(
        security,
        vec!["example.com".into(), "*.example.org".into()],
        Some("round16-browser".into()),
        "computer_use".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig {
            endpoint,
            api_key: Some("round16-sidecar-token".into()),
            timeout_ms: 1_000,
            allow_remote_endpoint: false,
            window_allowlist: vec!["OpenHuman".into()],
            max_coordinate_x: Some(100),
            max_coordinate_y: Some(100),
        },
    )
}

#[tokio::test]
async fn round16_browser_computer_use_validation_and_sidecar_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let state = SidecarState::default();
    let endpoint = start_computer_sidecar(state.clone()).await;
    let tool = browser_tool(endpoint, &harness.workspace);

    let ok = tool
        .execute(json!({ "action": "mouse_move", "x": 9, "y": 10 }))
        .await
        .expect("computer-use mouse_move");
    assert!(!ok.is_error, "{}", ok.output());
    assert!(ok.output().contains("\"echo_action\": \"mouse_move\""));

    let open = tool
        .execute(json!({ "action": "open", "url": "https://docs.example.org/path" }))
        .await
        .expect("computer-use open");
    assert!(!open.is_error, "{}", open.output());
    assert_eq!(state.requests.lock().expect("requests").len(), 2);

    for (args, expected) in [
        (
            json!({ "action": "mouse_click", "x": -1, "y": 2 }),
            "'x' must be >= 0",
        ),
        (
            json!({ "action": "mouse_drag", "from_x": 0, "from_y": 0, "to_x": 101, "to_y": 2 }),
            "exceeds configured limit",
        ),
        (
            json!({ "action": "open", "url": "file:///tmp/secret" }),
            "file:// URLs",
        ),
        (
            json!({ "action": "open", "url": "https://evil.test" }),
            "not in browser.allowed_domains",
        ),
        (json!({ "action": "definitely_missing" }), "Unknown action"),
    ] {
        let observed = match tool.execute(args).await {
            Ok(result) => {
                assert!(result.is_error);
                result.output().to_string()
            }
            Err(error) => error.to_string(),
        };
        assert!(
            observed.contains(expected),
            "expected {expected:?} in {observed}"
        );
    }

    let bad_endpoint = browser_tool("https://public.example.test/".into(), &harness.workspace)
        .execute(json!({ "action": "mouse_move", "x": 1, "y": 1 }))
        .await
        .expect("public endpoint is rejected as a tool result");
    assert!(bad_endpoint.is_error);
    assert!(bad_endpoint
        .output()
        .contains("host 'public.example.test' is public"));
}

#[test]
fn round16_all_tools_registry_branches_and_browser_allowlist() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let mut cfg = Config {
        workspace_dir: harness.workspace.clone(),
        config_path: harness.root.join("config.toml"),
        ..Config::default()
    };
    cfg.node.enabled = false;
    cfg.gitbooks.enabled = true;
    cfg.learning.enabled = true;
    cfg.learning.tool_tracking_enabled = true;
    cfg.browser.enabled = true;
    cfg.http_request.allowed_domains = vec![
        "*".to_string(),
        "example.com".to_string(),
        "*.example.org".to_string(),
    ];
    cfg.mcp_client.servers.push(McpServerConfig {
        name: "round16-docs".into(),
        endpoint: "https://example.com/mcp".into(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        cwd: None,
        description: Some("Round16 MCP".into()),
        enabled: true,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        timeout_secs: 10,
        auth: McpAuthConfig::None,
    });

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &Arc::new(SecurityPolicy::from_config(
            &cfg.autonomy,
            &harness.workspace,
            &harness.workspace,
        )),
        AuditLogger::disabled(),
        &BrowserConfig {
            enabled: true,
            session_name: Some("round16-session".into()),
            backend: "computer_use".into(),
            ..cfg.browser.clone()
        },
        &HttpRequestConfig {
            allowed_domains: cfg.http_request.allowed_domains.clone(),
            ..cfg.http_request.clone()
        },
        &harness.workspace,
        &HashMap::from([(
            "researcher".to_string(),
            openhuman_core::openhuman::config::DelegateAgentConfig {
                model: "round16-delegate-model".to_string(),
                system_prompt: Some("Delegate test prompt".to_string()),
                temperature: Some(0.0),
                max_depth: 1,
            },
        )]),
        &cfg,
    );
    let names = tool_names(&tools);

    for expected in [
        "spawn_subagent",
        "spawn_parallel_agents",
        "browser",
        "browser_open",
        "http_request",
        "web_fetch",
        "curl",
        "gitbooks_search",
        "gitbooks_get_page",
        "mcp_list_servers",
        "mcp_list_tools",
        "mcp_call_tool",
        "tool_stats",
        "delegate",
        "mcp_setup_search",
        "mcp_setup_install_and_connect",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "expected {expected} in {names:?}"
        );
    }
    assert!(!names.iter().any(|name| name == "mouse"));
    assert!(!names.iter().any(|name| name == "keyboard"));
    assert!(!names.iter().any(|name| name == "node_exec"));
    assert!(!names.iter().any(|name| name == "npm_exec"));
    assert!(
        names.iter().any(|name| name == "browser"),
        "browser registration covers http_request allowlist normalization"
    );
}

#[tokio::test]
async fn round16_spawn_subagent_tool_and_runner_error_success_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let tool = SpawnSubagentTool::new();

    let missing_agent = tool
        .execute(json!({ "prompt": "do work" }))
        .await
        .expect("missing agent returns tool result");
    assert!(missing_agent.is_error);
    assert!(missing_agent.output().contains("agent_id"));

    let disabled_thread = tool
        .execute(json!({
            "agent_id": "researcher",
            "prompt": "do work",
            "dedicated_thread": true
        }))
        .await
        .expect("dedicated thread returns tool result");
    assert!(
        disabled_thread.is_error,
        "dedicated_thread should error: {}",
        disabled_thread.output()
    );
    // #3049 superseded #1624: dedicated_thread is no longer "temporarily
    // disabled" — it's accepted but the tool may still error for other
    // reasons (e.g. no provider configured). Just verify it errors.
    assert!(
        !disabled_thread.output().is_empty(),
        "error output should not be empty"
    );

    let provider = Arc::new(ScriptedModel::new(vec![response(
        Some("subagent final answer that will be clipped"),
        Vec::new(),
    )]));
    let parent = parent_context(harness.workspace.clone(), provider.clone());
    let definition = agent_definition("round16_worker", Some(18));
    let outcome = with_parent_context(parent, async {
        run_subagent(
            &definition,
            "Summarize with no tools.",
            SubagentRunOptions {
                context: Some("caller context".into()),
                task_id: Some("round16-task".into()),
                ..Default::default()
            },
        )
        .await
    })
    .await
    .expect("run subagent with parent context");
    assert_eq!(outcome.agent_id, "round16_worker");
    assert_eq!(outcome.output, "subagent final ans\n[...truncated]");
    assert!(provider.requests()[0].iter().any(|message| {
        matches!(message, Message::User(_))
            && message.text().contains("parent memory")
            && message.text().contains("caller context")
    }));

    let no_parent = run_subagent(&definition, "no parent", SubagentRunOptions::default())
        .await
        .expect_err("subagent outside parent context fails")
        .to_string();
    assert!(no_parent.contains("no parent context"));
}

#[tokio::test]
async fn round16_agent_builder_turn_uses_public_harness_paths() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let provider = Arc::new(ScriptedModel::new(vec![
        response(
            Some("need echo"),
            vec![ToolCall::new(
                "call-round16",
                "echo",
                json!({ "message": "builder" }),
            )],
        ),
        response(Some("builder final"), Vec::new()),
    ]));
    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(vec![Box::new(EchoTool)])
        .memory(Arc::new(StubMemory))
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .config(openhuman_core::openhuman::config::AgentConfig {
            max_tool_iterations: 3,
            ..Default::default()
        })
        .model_name("round16-model".to_string())
        .temperature(0.0)
        .workspace_dir(harness.workspace.clone())
        .workflows(Vec::new())
        .auto_save(false)
        .event_context("round16-session", "round16-channel")
        .agent_definition_name("round16_builder")
        .omit_profile(true)
        .omit_memory_md(true)
        .build()
        .expect("agent builder");

    let answer = agent.turn("use echo once").await.expect("agent turn");
    assert_eq!(answer, "builder final");
    assert!(agent.history().iter().any(|message| matches!(
        message,
        openhuman_core::openhuman::agent::messages::ConversationMessage::ToolResults(results)
            if results.iter().any(|result| result.content.contains("echo:builder"))
    )));
}

#[test]
fn round16_auth_profiles_selection_migration_and_drop_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let state_dir = harness.root.join("profile-store");
    let store = AuthProfilesStore::new(&state_dir, false);

    let token = AuthProfile::new_token("channel:slack:bot", "primary", "xoxb-round16".into());
    store
        .upsert_profile(token.clone(), true)
        .expect("upsert token");

    let mut oauth = AuthProfile::new_oauth(
        "github",
        "work",
        TokenSet {
            access_token: "gh-round16".into(),
            refresh_token: Some("refresh-round16".into()),
            id_token: Some("id-round16".into()),
            expires_at: Some(Utc::now() + ChronoDuration::minutes(10)),
            token_type: Some("Bearer".into()),
            scope: Some("repo".into()),
        },
    );
    oauth.metadata = BTreeMap::from([("team".to_string(), "coverage".to_string())]);
    store
        .upsert_profile(oauth.clone(), false)
        .expect("upsert oauth");
    store
        .set_active_profile("github", &oauth.id)
        .expect("activate github");

    let loaded = store.load().expect("load auth profiles");
    assert_eq!(loaded.profiles[&oauth.id].kind, AuthProfileKind::OAuth);
    assert!(loaded.profiles[&oauth.id]
        .token_set
        .as_ref()
        .expect("token set")
        .is_expiring_within(std::time::Duration::from_secs(900)));

    let service = AuthService::new(&state_dir, false);
    assert_eq!(
        service
            .get_provider_bearer_token("github", None)
            .expect("github bearer")
            .as_deref(),
        Some("gh-round16")
    );
    assert!(service
        .set_active_profile("github", &token.id)
        .expect_err("wrong provider activation")
        .to_string()
        .contains("belongs to provider"));
    assert!(store
        .set_active_profile("github", "missing")
        .expect_err("missing active profile")
        .to_string()
        .contains("Auth profile not found"));

    let path = store.path().to_path_buf();
    let mut raw: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("profile json"))
            .expect("valid profile json");
    raw["schema_version"] = json!(0);
    raw["profiles"]["legacy-bad-kind"] = json!({
        "provider": "legacy",
        "profile_name": "bad",
        "kind": "api_key",
        "token": "legacy-token",
        "created_at": "not-a-date",
        "updated_at": "also-not-a-date",
        "metadata": {}
    });
    raw["active_profiles"]["legacy"] = json!("legacy-bad-kind");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&raw).expect("serialize"),
    )
    .expect("write bad kind");

    let migrated = store.load().expect("bad profile kind is dropped");
    assert_eq!(migrated.schema_version, 1);
    assert!(!migrated.profiles.contains_key("legacy-bad-kind"));
    assert!(!migrated.active_profiles.contains_key("legacy"));

    raw["schema_version"] = json!(999);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&raw).expect("serialize"),
    )
    .expect("write future schema");
    assert!(store
        .load()
        .expect_err("future schema rejected")
        .to_string()
        .contains("Unsupported auth profile schema version 999"));
}

#[tokio::test]
async fn round16_app_state_config_and_session_snapshot_edges() {
    let _lock = env_lock();
    let harness = setup("http://127.0.0.1:9");
    let config = harness.config().await;
    assert_eq!(
        config.default_model.as_deref(),
        Some("round16-coverage-model")
    );
    assert!(config.onboarding_completed);

    std::fs::create_dir_all(harness.app_state_file().parent().expect("state parent"))
        .expect("state dir");
    std::fs::write(harness.app_state_file(), "{broken").expect("write corrupt app state");

    let stored = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(Some("  round16-key  ".to_string())),
        onboarding_tasks: Some(Some(StoredOnboardingTasks {
            accessibility_permission_granted: true,
            local_model_consent_given: true,
            local_model_download_started: false,
            enabled_tools: vec!["gmail".to_string(), "slack".to_string()],
            connected_sources: vec!["github".to_string()],
            updated_at_ms: Some(16),
        })),
    })
    .await
    .expect("update app state")
    .value;
    assert_eq!(stored.encryption_key.as_deref(), Some("round16-key"));
    assert_eq!(
        stored
            .onboarding_tasks
            .as_ref()
            .expect("tasks")
            .connected_sources,
        vec!["github"]
    );

    let quarantined = std::fs::read_dir(harness.app_state_file().parent().expect("state parent"))
        .expect("state entries")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("app-state.json.corrupted")
        });
    assert!(quarantined, "corrupt app-state.json should be quarantined");

    let mut metadata = HashMap::new();
    metadata.insert("user_id".to_string(), "round16-user".to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "id": "round16-user",
            "displayName": "Round16 User",
            "email": "round16@example.test"
        })
        .to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round16.header.payload",
            metadata,
            true,
        )
        .expect("store app session");

    let snap = snapshot().await.expect("snapshot").value;
    assert!(snap.auth.is_authenticated);
    assert_eq!(
        snap.session_token.as_deref(),
        Some("round16.header.payload")
    );
    assert_eq!(snap.auth.user_id.as_deref(), Some("round16-user"));
    assert_eq!(
        snap.local_state.encryption_key.as_deref(),
        Some("round16-key")
    );

    let cleared = update_local_state(StoredAppStatePatch {
        keyring_consent: None,
        encryption_key: Some(None),
        onboarding_tasks: Some(None),
    })
    .await
    .expect("clear local app state")
    .value;
    assert!(cleared.encryption_key.is_none());
    assert!(cleared.onboarding_tasks.is_none());
}

/// #5847 — the one behaviour deliberately kept when TinyPlace was deleted.
///
/// The removal took ~49,600 lines but retained a single entry on the
/// workspace-internal denylist, because an upgraded profile can still hold
/// `tinyplace/` state written by an older version — encrypted identity and
/// session material. Nothing in any e2e lane asserted it, which makes the entry
/// look like a leftover reference to a deleted domain rather than the guard it
/// is: exactly the line a future cleanup deletes as dead.
///
/// Driven through the agent file tools rather than `is_workspace_internal_path`
/// directly, because the question is not whether the predicate is right — the
/// unit suite covers that — but whether the tools an agent actually calls sit
/// behind it, at the most permissive autonomy tier there is.
///
/// `redirect_links` and `codegraph` are asserted alongside it: all three are
/// retained-after-removal entries with the same rationale, so one regression
/// would take all three and a test naming only `tinyplace` would miss it.
#[tokio::test]
async fn file_tools_cannot_reach_retained_legacy_state_in_the_workspace() {
    use openhuman_core::openhuman::config::AutonomyConfig;
    use openhuman_core::openhuman::security::AutonomyLevel;
    use openhuman_core::openhuman::tools::{FileReadTool, FileWriteTool};

    let tmp = tempdir();
    let workspace = tmp.path().to_path_buf();
    std::fs::create_dir_all(&workspace).expect("workspace");

    // Full autonomy, and the workspace is also the action dir — the most
    // permissive shape a real install can take. If the guard holds here it
    // holds everywhere.
    let security = Arc::new(SecurityPolicy::from_config(
        &AutonomyConfig {
            level: AutonomyLevel::Full,
            max_actions_per_hour: 10_000,
            ..Default::default()
        },
        &workspace,
        &workspace,
    ));
    let read = FileReadTool::new(security.clone());
    let write = FileWriteTool::new(security.clone());

    // Paths are workspace-relative on purpose: `workspace_only` is on by
    // default and rejects every absolute path outright
    // (`security/policy/path_checks.rs:88`), so an absolute path would be
    // refused for a reason that has nothing to do with the denylist under test.
    // Relative is also what an agent actually sends — file tools resolve them
    // from `action_dir`.
    //
    // A control: an ordinary workspace file must stay reachable, so a passing
    // test cannot be explained by everything being blocked.
    std::fs::write(workspace.join("notes.md"), "reachable\n").expect("write control file");
    let control = read
        .execute(json!({"path": "notes.md"}))
        .await
        .expect("control read");
    assert!(
        !control.is_error && control.output().contains("reachable"),
        "an ordinary workspace file must remain readable: {}",
        control.output()
    );

    for legacy in ["tinyplace", "redirect_links", "codegraph"] {
        let dir = workspace.join(legacy);
        std::fs::create_dir_all(&dir).expect("legacy dir");
        let secret = dir.join("session.json");
        let original = format!("{{\"legacy\":\"{legacy}\",\"token\":\"do-not-read\"}}");
        std::fs::write(&secret, &original).expect("seed legacy state");

        let relative = format!("{legacy}/session.json");
        let attempted_read = read
            .execute(json!({"path": relative}))
            .await
            .expect("read returns a ToolResult");
        assert!(
            attempted_read.is_error,
            "{legacy}: agent read of retained legacy state succeeded: {}",
            attempted_read.output()
        );
        assert!(
            !attempted_read.output().contains("do-not-read"),
            "{legacy}: the secret leaked into the tool output: {}",
            attempted_read.output()
        );

        let attempted_write = write
            .execute(json!({"path": format!("{legacy}/session.json"), "content": "clobbered"}))
            .await
            .expect("write returns a ToolResult");
        assert!(
            attempted_write.is_error,
            "{legacy}: agent write into retained legacy state succeeded: {}",
            attempted_write.output()
        );
        // The refusal must not itself disclose what it is protecting. A write
        // path that echoes the existing file back in its error would satisfy
        // the flag and the byte-comparison below while still leaking the
        // secret (CodeRabbit, #5974).
        assert!(
            !attempted_write.output().contains("do-not-read"),
            "{legacy}: the write refusal leaked the file it refused to touch: {}",
            attempted_write.output()
        );
        // The refusal has to be a refusal, not a message printed after the fact.
        assert_eq!(
            std::fs::read_to_string(&secret).expect("legacy file still present"),
            original,
            "{legacy}: the file was modified despite the tool reporting an error"
        );
    }
}

/// #5807 — a driver deliberately given `class = "null"` is named as such, not
/// blamed on a build that has no memory module.
///
/// `admit` accepts a non-built-in driver id carrying `class = "null"` verbatim
/// (the `built_in_class` consistency check is skipped for ids that are not
/// built in), so the binding reports `class = Null` with a `driver_id` that is
/// not `"null"`. Keying the refusal reason on that id put this config in the
/// modules-off arm and told a user with a working modules build to go and
/// investigate their build flags.
///
/// Driven through `migration_helpers::rpc::migrate_openclaw` — the function the
/// `config.openclaw` RPC handler calls — rather than the private wrapper the
/// unit test uses, and with an explicit `Config` so the assertion does not
/// depend on process-global config state shared with the rest of this binary.
///
/// This runs in every feature configuration on purpose: the misdirection it
/// guards against is one a *modules-enabled* build hits.
#[tokio::test]
async fn openclaw_import_names_a_null_classed_driver_rather_than_the_build() {
    use openhuman_core::openhuman::config::migration_helpers::rpc as migration_rpc;
    use openhuman_core::openhuman::config::schema::{MemoryDriverConfig, MemorySubsystemConfig};

    let tmp = tempdir();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");

    let mut drivers = std::collections::BTreeMap::new();
    drivers.insert(
        "mynull".to_string(),
        MemoryDriverConfig {
            class: Some("null".to_string()),
            ..Default::default()
        },
    );
    config.subsystems.memory = MemorySubsystemConfig {
        driver: "mynull".to_string(),
        drivers,
        ..Default::default()
    };

    let source = tmp.path().join("openclaw-src");
    std::fs::create_dir_all(&source).expect("source workspace");
    std::fs::write(source.join("MEMORY.md"), "# Note\nkeep me").expect("seed source");

    let err = migration_rpc::migrate_openclaw(&config, Some(source), false)
        .await
        .expect_err("importing into a null-classed driver must refuse");

    // `contains("null")` alone would be satisfied by the driver id `mynull`,
    // so this asserts the *quoted* class value the message renders — which a
    // generic class-validation error could not produce (CodeRabbit, #5974).
    assert!(
        err.contains("mynull") && err.contains("class") && err.contains("\"null\""),
        "the refusal must name the configured driver and its `class = \"null\"`; got: {err}"
    );
    assert!(
        !err.contains("no memory module compiled in"),
        "a deliberately null-classed driver must not be reported as a modules-off \
         build — that is the misdirection #5807 fixed; got: {err}"
    );
}
