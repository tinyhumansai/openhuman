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
use openhuman_core::openhuman::app_state::{
    snapshot, update_local_state, StoredAppStatePatch, StoredOnboardingTasks,
};
use openhuman_core::openhuman::config::rpc as config_rpc;
use openhuman_core::openhuman::config::{
    BrowserConfig, Config, HttpRequestConfig, McpAuthConfig, McpServerConfig,
};
use openhuman_core::openhuman::context::prompt::ToolCallFormat;
use openhuman_core::openhuman::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary};
use openhuman_core::openhuman::security::{AuditLogger, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    all_tools, BrowserTool, ComputerUseConfig, SpawnSubagentTool, Tool, ToolResult,
};
use parking_lot::Mutex as ParkingMutex;
use serde_json::{json, Value};
use tempfile::{Builder, TempDir};

static ROUND16_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

struct ScriptedProvider {
    responses: ParkingMutex<Vec<ChatResponse>>,
    requests: ParkingMutex<Vec<Vec<ChatMessage>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: ParkingMutex::new(responses),
            requests: ParkingMutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Vec<ChatMessage>> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok(format!("extract:{message}"))
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().push(request.messages.to_vec());
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

fn usage(input_tokens: u64, output_tokens: u64) -> UsageInfo {
    UsageInfo {
        input_tokens,
        output_tokens,
        context_window: 8_192,
        cached_input_tokens: input_tokens / 2,
        charged_amount_usd: 0.001,
    }
}

fn response(text: Option<&str>, tool_calls: Vec<ToolCall>) -> ChatResponse {
    ChatResponse {
        text: text.map(str::to_string),
        tool_calls,
        usage: Some(usage(50, 7)),
        reasoning_content: None,
    }
}

fn parent_context(workspace: PathBuf, provider: Arc<ScriptedProvider>) -> ParentExecutionContext {
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
        provider,
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(tool_specs),
        model_name: "round16-model".to_string(),
        temperature: 0.0,
        workspace_dir: workspace,
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
        omit_skills_catalog: true,
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
        timeout_secs: None,
        sandbox_mode: SandboxMode::ReadOnly,
        background: false,
        trigger_memory_agent: Default::default(),
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: Default::default(),
        source: Default::default(),
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
        .execute(json!({ "action": "screen_capture" }))
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
    cfg.computer_control.enabled = true;
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
        Arc::new(StubMemory),
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
        "mouse",
        "keyboard",
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

    let provider = Arc::new(ScriptedProvider::new(vec![response(
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
        message.role == "user"
            && message.content.contains("parent memory")
            && message.content.contains("caller context")
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
    let provider = Arc::new(ScriptedProvider::new(vec![
        response(
            Some("need echo"),
            vec![ToolCall {
                id: "call-round16".into(),
                name: "echo".into(),
                arguments: json!({ "message": "builder" }).to_string(),
                extra_content: None,
            }],
        ),
        response(Some("builder final"), Vec::new()),
    ]));
    let mut agent = Agent::builder()
        .provider_arc(provider)
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
        openhuman_core::openhuman::inference::provider::ConversationMessage::ToolResults(results)
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
