//! The capability seam: five adapters implementing `tinyflows::caps` traits
//! over real OpenHuman services.
//!
//! Each tinyflows integration node hands its **whole** `node.config` to the
//! matching trait method — the adapter interprets a free-form JSON value the
//! flow author wrote, pulling a connection ref out of `config["connection_ref"]`
//! where relevant. See `my_docs/ohxtf/b1-engine-seam-domain/04-capability-seam.md`
//! for the source-verified node → trait contract this mirrors.
//!
//! All host errors are mapped to `tinyflows::error::EngineError::Capability`,
//! per the crate's contract (`caps` traits return `tinyflows::error::Result`).

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents::graph::SqliteCheckpointer;
use tinyflows::caps::{
    Capabilities, CodeLanguage, CodeRunner, HttpClient, LlmProvider, StateStore, ToolInvoker,
};
use tinyflows::error::{EngineError, Result};

use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::composio::client::{
    create_composio_client, direct_execute, ComposioClientKind,
};
use crate::openhuman::config::{Config, HttpRequestConfig};
use crate::openhuman::flows;
use crate::openhuman::inference::provider::{
    create_chat_provider, ChatMessage, ChatRequest, UsageInfo,
};
use crate::openhuman::sandbox::{execute_in_sandbox, resolve_sandbox_policy};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::Tool as _;
use crate::openhuman::tools::HttpRequestTool;

/// Maps a `UsageInfo` (not `Serialize`) into a JSON value field-by-field, so
/// [`OpenHumanLlm::complete`] can surface it in its response `Value` without
/// requiring an upstream `Serialize` impl change.
fn usage_to_json(usage: &Option<UsageInfo>) -> Value {
    match usage {
        None => Value::Null,
        Some(u) => json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "context_window": u.context_window,
            "cached_input_tokens": u.cached_input_tokens,
            "cache_creation_tokens": u.cache_creation_tokens,
            "reasoning_tokens": u.reasoning_tokens,
            "charged_amount_usd": u.charged_amount_usd,
        }),
    }
}

/// [`LlmProvider`] adapter over OpenHuman's inference stack
/// (`src/openhuman/inference/provider/`).
///
/// The `agent` node is single-completion in tinyflows 0.2 (no tool-calling
/// loop, no sub-ports), so `complete` performs exactly one `provider.chat`
/// call and returns its result — no agent loop is driven here.
pub struct OpenHumanLlm {
    pub config: Arc<Config>,
}

#[async_trait]
impl LlmProvider for OpenHumanLlm {
    async fn complete(&self, request: Value, conn: Option<&str>) -> Result<Value> {
        if let Some(c) = conn {
            // B1 does not resolve `connection_ref` to a specific BYOK account —
            // `create_chat_provider` picks the configured provider for `role`.
            tracing::debug!(target: "flows", conn = %c, "[flows] llm conn (not resolved in B1)");
        }

        let role = request
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("summarization");
        let temperature = request
            .get("temperature")
            .and_then(Value::as_f64)
            .unwrap_or(0.7);
        let max_tokens = request
            .get("max_tokens")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());

        let messages: Vec<ChatMessage> = match request.get("messages").and_then(Value::as_array) {
            Some(entries) if !entries.is_empty() => entries
                .iter()
                .filter_map(|entry| {
                    let content = entry.get("content").and_then(Value::as_str)?.to_string();
                    let role = entry.get("role").and_then(Value::as_str).unwrap_or("user");
                    Some(match role {
                        "system" => ChatMessage::system(content),
                        "assistant" => ChatMessage::assistant(content),
                        "tool" => ChatMessage::tool(content),
                        _ => ChatMessage::user(content),
                    })
                })
                .collect(),
            _ => {
                let prompt = request
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                vec![ChatMessage::user(prompt)]
            }
        };

        tracing::debug!(
            target: "flows",
            role,
            message_count = messages.len(),
            "[flows] llm.complete: dispatching agent-node completion"
        );

        let (provider, model) = create_chat_provider(role, &self.config)
            .map_err(|e| EngineError::Capability(e.to_string()))?;

        let response = provider
            .chat(
                ChatRequest {
                    messages: &messages,
                    tools: None,
                    stream: None,
                    max_tokens,
                },
                &model,
                temperature,
            )
            .await
            .map_err(|e| EngineError::Capability(e.to_string()))?;

        Ok(json!({
            "text": response.text,
            "tool_calls": response.tool_calls,
            "usage": usage_to_json(&response.usage),
            "reasoning_content": response.reasoning_content,
        }))
    }
}

/// [`ToolInvoker`] adapter over Composio (`src/openhuman/composio/client.rs`).
///
/// **B1 deviation (tracked, see `my_docs/ohxtf/commons/11-gotchas-and-decisions.md`):**
/// `connection_ref` is logged but not forwarded — `execute_tool` (backend mode)
/// takes no connection id and resolves the ambient signed-in account; direct
/// mode uses `config.composio.entity_id`. Fine for a single-account desktop
/// user; must be resolved before multi-account or B2 trigger runs. There is
/// also no curated-tool-set / scope filter yet — `invoke` will call any slug.
pub struct OpenHumanTools {
    pub config: Arc<Config>,
}

#[async_trait]
impl ToolInvoker for OpenHumanTools {
    async fn invoke(&self, slug: &str, args: Value, conn: Option<&str>) -> Result<Value> {
        if let Some(c) = conn {
            tracing::debug!(
                target: "flows",
                %slug,
                conn = %c,
                "[flows] tool conn (backend resolves ambient account; not forwarded in B1)"
            );
        }

        let kind = create_composio_client(&self.config)
            .map_err(|e| EngineError::Capability(e.to_string()))?;
        let args_opt = if args.is_null() { None } else { Some(args) };

        tracing::debug!(target: "flows", %slug, mode = kind.mode(), "[flows] tool_call: invoking composio tool");

        let response = match kind {
            ComposioClientKind::Backend(client) => client
                .execute_tool(slug, args_opt)
                .await
                .map_err(|e| EngineError::Capability(e.to_string()))?,
            ComposioClientKind::Direct(tool) => {
                direct_execute(&tool, slug, args_opt, &self.config.composio.entity_id, None)
                    .await
                    .map_err(|e| EngineError::Capability(e.to_string()))?
            }
        };

        serde_json::to_value(response).map_err(|e| EngineError::Capability(e.to_string()))
    }
}

/// [`HttpClient`] adapter over `HttpRequestTool`
/// (`src/openhuman/tools/impl/network/http_request.rs`). Allowlist + DNS-rebind
/// guard + Network gating live inside `execute`, so this adapter gets them for
/// free.
pub struct OpenHumanHttp {
    pub security: Arc<SecurityPolicy>,
    pub http_config: HttpRequestConfig,
}

#[async_trait]
impl HttpClient for OpenHumanHttp {
    async fn request(&self, request: Value, conn: Option<&str>) -> Result<Value> {
        if let Some(c) = conn {
            tracing::debug!(target: "flows", conn = %c, "[flows] http conn (not resolved in B1)");
        }

        let tool = HttpRequestTool::new(
            self.security.clone(),
            self.http_config.allowed_domains.clone(),
            self.http_config.max_response_size,
            self.http_config.timeout_secs,
        );

        tracing::debug!(
            target: "flows",
            method = ?request.get("method"),
            url = ?request.get("url"),
            "[flows] http_request: dispatching outbound request"
        );

        // `request` is already `{ method, url, headers?, body? }` — the node's
        // config is the request descriptor; `HttpRequestTool::execute` reads
        // only those keys and ignores the rest (e.g. `connection_ref`,
        // `on_error`), so passing the whole config through is safe.
        let result = tool
            .execute(request)
            .await
            .map_err(|e| EngineError::Capability(e.to_string()))?;

        // `HttpRequestTool::execute` always returns `Ok`, using `is_error` to
        // signal a failed request (non-2xx, DNS/allowlist rejection, timeout,
        // …) — surface that as a capability error so the engine's
        // `on_error`/`retry` policy can act on it.
        if result.is_error {
            return Err(EngineError::Capability(result.text()));
        }

        Ok(json!({ "text": result.text() }))
    }
}

/// [`CodeRunner`] adapter running sandboxed user code via
/// `src/openhuman/sandbox/ops.rs` (`resolve_sandbox_policy` +
/// `execute_in_sandbox`), modeled on
/// `src/openhuman/tools/impl/system/node_exec.rs::run_sandboxed`.
///
/// **Mismatch handled here:** the sandbox runs a shell command string, not a
/// `(language, source, input)` triple. `source` is treated as a function body
/// receiving the serialized `input` items array and returning the node's
/// output — this convention is a B1 design choice (not specified by the
/// crate), matching the mock's "function body" tests
/// (`tinyflows::nodes::integration::code` — e.g. `"source": "return 1;"`).
///
/// Requires `node`/`python3` on the `PATH` the sandbox backend runs under;
/// there is no managed toolchain wiring here (unlike `node_exec`'s
/// `NodeBootstrap`).
pub struct OpenHumanCode {
    pub config: Arc<Config>,
}

const CODE_RUN_TIMEOUT_SECS: u64 = 60;

#[async_trait]
impl CodeRunner for OpenHumanCode {
    async fn run(&self, language: CodeLanguage, source: &str, input: Value) -> Result<Value> {
        let policy = resolve_sandbox_policy(
            SandboxMode::Sandboxed,
            &self.config.action_dir,
            &self.config.runtime,
            false,
        );

        // Work dir lives under `action_dir` (the sandbox workspace root). We keep
        // its path *relative* to `action_dir` so the run command works on every
        // backend: for Local, `execute_in_sandbox`'s `working_dir` is the host
        // cwd; for Docker, `action_dir` is bind-mounted at `/workspace` with
        // `-w /workspace`. Host-absolute paths would not exist inside the
        // container, so we pass `action_dir` as the working dir and reference the
        // script/input by their `action_dir`-relative paths.
        let rel_dir = std::path::Path::new(".flows_code").join(uuid::Uuid::new_v4().to_string());
        let work_dir = self.config.action_dir.join(&rel_dir);
        tokio::fs::create_dir_all(&work_dir)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to create code work dir: {e}")))?;

        let (script_name, interpreter, script_body) = match language {
            CodeLanguage::JavaScript => ("script.js", "node", js_harness(source)),
            CodeLanguage::Python => ("script.py", "python3", python_harness(source)),
        };
        let script_path = work_dir.join(script_name);
        let input_path = work_dir.join("input.json");

        let input_json = serde_json::to_string(&input)
            .map_err(|e| EngineError::Capability(format!("failed to serialize code input: {e}")))?;
        tokio::fs::write(&script_path, script_body)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to write code script: {e}")))?;
        tokio::fs::write(&input_path, input_json)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to write code input: {e}")))?;

        // Backend-agnostic, `action_dir`-relative command paths (see above).
        let rel_script = rel_dir.join(script_name);
        let rel_input = rel_dir.join("input.json");
        let command = format!(
            "{} {} {}",
            shell_quote(interpreter),
            shell_quote(&rel_script.to_string_lossy()),
            shell_quote(&rel_input.to_string_lossy()),
        );

        let mut extra_env = std::collections::HashMap::new();
        if let Ok(host_path) = std::env::var("PATH") {
            extra_env.insert("PATH".to_string(), host_path);
        }

        tracing::debug!(
            target: "flows",
            ?language,
            work_dir = %work_dir.display(),
            "[flows] code: running sandboxed script"
        );

        let exec_result = execute_in_sandbox(
            &policy,
            &command,
            &self.config.action_dir,
            extra_env,
            std::time::Duration::from_secs(CODE_RUN_TIMEOUT_SECS),
        )
        .await;

        // Always clean up the work dir — even when `execute_in_sandbox` itself
        // errors (e.g. a spawn failure) — so temp scripts never leak.
        if let Err(e) = tokio::fs::remove_dir_all(&work_dir).await {
            tracing::debug!(target: "flows", error = %e, "[flows] code: failed to clean up work dir (non-fatal)");
        }

        let result = exec_result
            .map_err(|e| EngineError::Capability(format!("sandbox execution failed: {e}")))?;

        if !result.success() {
            return Err(EngineError::Capability(format!(
                "code node exited non-zero (timed_out={}): {}",
                result.timed_out, result.stderr
            )));
        }

        serde_json::from_str(result.stdout.trim())
            .map_err(|e| EngineError::Capability(format!("code output was not valid JSON: {e}")))
    }
}

/// Wraps user `source` as a function body receiving `input`, executed by Node,
/// printing the JSON result (or `null`) to stdout.
fn js_harness(source: &str) -> String {
    format!(
        "const fs = require('fs');\n\
         const input = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         const __result__ = (function(input) {{\n{source}\n}})(input);\n\
         process.stdout.write(JSON.stringify(__result__ === undefined ? null : __result__));\n"
    )
}

/// Wraps user `source` as a function body receiving `input`, executed by
/// Python, printing the JSON result (or `null`) to stdout.
fn python_harness(source: &str) -> String {
    let indented: String = if source.trim().is_empty() {
        "    pass".to_string()
    } else {
        source
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "import sys, json\n\
         with open(sys.argv[1]) as __f__:\n    input = json.load(__f__)\n\
         def __user_fn__(input):\n{indented}\n    return None\n\
         __result__ = __user_fn__(input)\n\
         print(json.dumps(__result__))\n"
    )
}

/// POSIX single-quote shell escaping, mirroring
/// `tools/impl/system/node_exec.rs::shell_quote`.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// [`StateStore`] adapter over the `flows::` domain's `flow_state` KV table.
pub struct FlowStateStore {
    pub config: Arc<Config>,
    pub namespace: String,
}

#[async_trait]
impl StateStore for FlowStateStore {
    async fn load(&self, key: &str) -> Result<Option<Value>> {
        flows::kv_get(&self.config, &self.namespace, key)
            .map_err(|e| EngineError::Capability(e.to_string()))
    }

    async fn store(&self, key: &str, value: Value) -> Result<()> {
        flows::kv_set(&self.config, &self.namespace, key, &value)
            .map_err(|e| EngineError::Capability(e.to_string()))
    }
}

/// Builds the [`Capabilities`] bundle for one run, wiring each of the five
/// host-injected traits to a real OpenHuman adapter (see each adapter above for
/// its contract).
///
/// `state_namespace` scopes the [`FlowStateStore`] KV so two saved flows that
/// use the same state key never read or overwrite each other — callers pass a
/// per-flow namespace (e.g. `"flow:<id>"`).
pub fn build_capabilities(config: Arc<Config>, state_namespace: impl Into<String>) -> Capabilities {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let http_config = config.http_request.clone();

    Capabilities {
        llm: Arc::new(OpenHumanLlm {
            config: config.clone(),
        }),
        tools: Arc::new(OpenHumanTools {
            config: config.clone(),
        }),
        http: Arc::new(OpenHumanHttp {
            security,
            http_config,
        }),
        code: Arc::new(OpenHumanCode {
            config: config.clone(),
        }),
        state: Arc::new(FlowStateStore {
            config,
            namespace: state_namespace.into(),
        }),
    }
}

/// Opens the durable, cross-process checkpointer a `flows_run` uses via
/// `tinyflows::engine::run_with_checkpointer` — the crate's own
/// `tinyagents::graph::SqliteCheckpointer`, stored under
/// `<workspace_dir>/flows/checkpoints.db`.
///
/// Deliberately **not** a bespoke checkpointer: the crate ships its own
/// SQLite-backed `Checkpointer<State>` impl (feature `sqlite`, already enabled
/// on the `tinyagents` dependency), so the seam just opens it — mirrors the
/// construction in `src/openhuman/agent_orchestration/delegation.rs`.
pub fn open_flow_checkpointer(
    config: &Config,
) -> anyhow::Result<Arc<dyn tinyflows::engine::Checkpointer<serde_json::Value>>> {
    let db_path = config.workspace_dir.join("flows").join("checkpoints.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create flows directory: {}", parent.display()))?;
    }
    tracing::debug!(target: "flows", db = %db_path.display(), "[flows] opening checkpointer");
    Ok(Arc::new(
        SqliteCheckpointer::<serde_json::Value>::open(&db_path)
            .with_context(|| format!("Failed to open flows checkpointer: {}", db_path.display()))?,
    ))
}
