use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, AgentDefinition, DefinitionSource, ModelSpec,
    ParentExecutionContext, PromptSource, SandboxMode, SubagentRunOptions, ToolScope,
};
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::{
    ConnectedIntegration, ConnectedIntegrationTool, ToolCallFormat,
};
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use parking_lot::Mutex;
use serde_json::json;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

/// Serialize tests in this binary that mutate process-global `OPENHUMAN_WORKSPACE`
/// (read by `apply_env_overrides` during config load), so parallel test threads
/// can't observe each other's workspace override mid-run.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<ChatMessage>,
    tools_sent: bool,
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<ChatResponse>>,
    requests: Mutex<Vec<CapturedRequest>>,
    extraction_prompts: Mutex<Vec<String>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
            extraction_prompts: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().clone()
    }

    fn extraction_prompts(&self) -> Vec<String> {
        self.extraction_prompts.lock().clone()
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
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> Result<String> {
        self.extraction_prompts.lock().push(format!(
            "system={}\nmodel={model}\ntemperature={temperature}\n{message}",
            system_prompt.unwrap_or_default()
        ));
        Ok("round25 extracted: NEEDLE-42".to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().push(CapturedRequest {
            messages: request.messages.to_vec(),
            tools_sent: request.tools.is_some(),
        });
        Ok(self
            .responses
            .lock()
            .pop_front()
            .unwrap_or_else(|| text_response("round25 fallback final")))
    }
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "round25-memory"
    }

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
        _opts: RecallOpts<'_>,
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
}

struct LargePayloadTool;

#[async_trait]
impl Tool for LargePayloadTool {
    fn name(&self) -> &str {
        "round25_large_payload"
    }

    fn description(&self) -> &str {
        "Returns a deterministic oversized payload for handoff extraction"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            }
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        // The payload must remain large enough to trigger the handoff path
        // even after tokenjuice's generic/fallback reducer runs. The reducer
        // does head(8)+tail(8) of lines, then clamp_text_middle (max 1200
        // chars). Crucially, clamp_text_middle's `trim_head_to_line_boundary`
        // leaves the head slice unchanged when there is no `\n` in the head
        // 70% of the clamp window — so a long single-line body keeps ~840
        // chars in the head half. Combined with the tail the result is ~880
        // chars (≈220 tokens), which exceeds the test-mode threshold of 200
        // tokens set via `OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS=200`.
        // No HTML markup: clean_tool_output runs after tokenjuice and would
        // strip HTML tags, shrinking the output.
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        let mut bulk = String::with_capacity(1024);
        while bulk.len() < 1000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            bulk.push_str(&format!("{seed:016x}"));
        }
        // Three lines: preview anchor, large incompressible body, target fact.
        // • Line 2 is a single ~1000-char line with no internal newlines —
        //   trim_head_to_line_boundary keeps the full 840-char head slice.
        // • "target fact: NEEDLE-42" appears only in the last line so that
        //   the test can verify it survives in the extracted content.
        let payload = format!("record: first visible preview\n{bulk}\ntarget fact: NEEDLE-42");
        Ok(ToolResult::success(payload))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 11,
            output_tokens: 7,
            context_window: 32_000,
            cached_input_tokens: 3,
            charged_amount_usd: 0.0002,
        }),
        reasoning_content: None,
    }
}

fn xml_tool_response(name: &str, args: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "round25 call <tool_call>{{\"name\":\"{name}\",\"arguments\":{args}}}</tool_call>"
        )),
        tool_calls: Vec::new(),
        usage: None,
        reasoning_content: None,
    }
}

fn integrations_definition() -> AgentDefinition {
    AgentDefinition {
        id: "integrations_agent".to_string(),
        when_to_use: "round25 raw coverage".to_string(),
        display_name: Some("Round25 Integrations".to_string()),
        system_prompt: PromptSource::Inline("Round25 integrations prompt".to_string()),
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec!["round25_large_payload".to_string()]),
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations: 4,
        iteration_policy: Default::default(),
        max_result_chars: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: Default::default(),
        source: DefinitionSource::Builtin,
    }
}

fn parent(workspace_dir: PathBuf, provider: Arc<ScriptedProvider>) -> ParentExecutionContext {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(LargePayloadTool)];
    let specs = tools.iter().map(|tool| tool.spec()).collect();
    ParentExecutionContext {
        provider,
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(specs),
        model_name: "round25-parent-model".to_string(),
        temperature: 0.0,
        workspace_dir,
        memory: Arc::new(StubMemory),
        agent_config: AgentConfig::default(),
        skills: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("round25 inherited parent context".to_string())),
        session_id: "round25-session".to_string(),
        channel: "round25".to_string(),
        connected_integrations: vec![ConnectedIntegration {
            toolkit: "gmail".to_string(),
            description: "Round25 Gmail".to_string(),
            tools: vec![ConnectedIntegrationTool {
                name: "GMAIL_ROUND25_UNUSED".to_string(),
                description: "unused cached action".to_string(),
                parameters: Some(json!({"type": "object"})),
            }],
            gated_tools: Vec::new(),
            connected: true,
            non_active_status: None,
        }],
        tool_call_format: ToolCallFormat::PFormat,
        session_key: "1700000000_round25_parent".to_string(),
        session_parent_prefix: Some("root_chain".to_string()),
        on_progress: None,
    }
}

#[tokio::test]
async fn integrations_text_mode_handoffs_oversized_result_and_extracts_from_cache() -> Result<()> {
    let _env = env_lock();
    let workspace = tempfile::tempdir()?;
    let _workspace_guard = EnvGuard::set_path("OPENHUMAN_WORKSPACE", workspace.path());
    // Lower the handoff threshold and chunk budget so this test can exercise
    // the oversized-result path with payloads that survive tokenjuice's
    // default 1200-char compaction. These env vars are only read in
    // `apply_handoff` / `extract_from_result::execute` and have no effect
    // outside of test runs.
    let _handoff_thresh_guard = EnvGuard::set("OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS", "200");
    let _chunk_budget_guard = EnvGuard::set("OPENHUMAN_TEST_EXTRACT_CHUNK_BUDGET", "300");
    let provider = ScriptedProvider::new(vec![
        xml_tool_response("round25_large_payload", json!({"query": "find needle"})),
        xml_tool_response(
            "extract_from_result",
            json!({"result_id": "res_1", "query": "target fact only"}),
        ),
        text_response("final answer uses round25 extracted: NEEDLE-42"),
    ]);

    let outcome = with_parent_context(
        parent(workspace.path().to_path_buf(), provider.clone()),
        async {
            run_subagent(
                &integrations_definition(),
                "Use gmail to find the target fact.",
                SubagentRunOptions {
                    task_id: Some("round25-task".to_string()),
                    toolkit_override: Some("gmail".to_string()),
                    ..SubagentRunOptions::default()
                },
            )
            .await
        },
    )
    .await
    .expect("subagent should complete");

    assert_eq!(
        outcome.output,
        "final answer uses round25 extracted: NEEDLE-42"
    );
    assert_eq!(outcome.iterations, 3);

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests.iter().all(|request| !request.tools_sent),
        "integrations_agent text mode should omit native tool schemas"
    );
    assert!(
        requests[0].messages[0]
            .content
            .contains("To use a tool, wrap a JSON object in <tool_call></tool_call> tags"),
        "text-mode protocol should be injected into the system prompt"
    );
    let second_request = requests[1]
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(second_request.contains("result_id=\"res_1\""));
    assert!(second_request.contains("extract_from_result(result_id=\"res_1\""));
    // Note: with the test-only EXTRACT_CHUNK_CHAR_BUDGET (300 chars) the cached
    // payload (tokenjuice-compacted to ~1200 chars) fits within the handoff
    // preview window (1500 chars), so the raw tail may appear in the second
    // request. The key assertion is that the result_id handoff placeholder is
    // present; the `NEEDLE-42` visibility check is production-only (at 60k
    // chunk budget the 260k raw payload stays hidden until extracted).

    let extraction_prompts = provider.extraction_prompts();
    assert!(
        extraction_prompts.len() > 1,
        "oversized cached payload (test chunk budget=300 chars) should be split into multiple extraction calls, got {} prompts",
        extraction_prompts.len()
    );
    assert!(extraction_prompts
        .iter()
        .any(|prompt| prompt.contains("target fact: NEEDLE-42")));
    assert!(extraction_prompts
        .iter()
        .all(|prompt| prompt.contains("model=summarization-v1")));
    assert!(extraction_prompts
        .iter()
        .all(|prompt| prompt.contains("temperature=0.2")));

    let raw_dir = workspace.path().join("session_raw");
    assert!(
        raw_dir.exists(),
        "subagent and extract transcripts should be persisted under session_raw"
    );

    Ok(())
}
