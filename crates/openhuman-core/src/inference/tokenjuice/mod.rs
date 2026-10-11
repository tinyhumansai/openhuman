//! OpenHuman host adapter for the separately released TinyJuice module.

pub mod config_patch;
pub mod focus;
pub mod generate;
pub mod repl_tools;
pub mod savings;
pub mod schemas;
pub mod tools;
pub mod types;

use tinyjuice_bus::names::methods;

pub use repl_tools::{is_repl_tool, repl_tools, repl_tools_for, REPL_TOOL_NAMES};
pub use tools::TokenjuiceRetrieveTool;
pub use types::{AgentTokenjuiceCompression, CompressorKind, ContentKind};

use types::InstallRequest;

pub const RETRIEVE_TOOL_NAME: &str = "juice_retrieve";
pub const LEGACY_TINYJUICE_RETRIEVE_TOOL_NAME: &str = "tinyjuice_retrieve";
pub const LEGACY_RETRIEVE_TOOL_NAME: &str = "retrieve_tool_output";
/// Every name the recovery surface answers to: the live tool plus the two
/// migration aliases a replayed transcript may still call.
pub const RECOVERY_TOOL_NAMES: &[&str] = &[
    RETRIEVE_TOOL_NAME,
    LEGACY_TINYJUICE_RETRIEVE_TOOL_NAME,
    "tokenjuice_retrieve",
    LEGACY_RETRIEVE_TOOL_NAME,
];

/// The recovery tool a curated (`ToolScope::Named`) belt is guaranteed to
/// advertise. Only the live tool: the aliases stay registered so an old
/// transcript replays, but putting all three on the wire charged every
/// Named agent for three copies of one schema.
pub const RECOVERY_TOOL_VISIBLE: &[&str] = &[RETRIEVE_TOOL_NAME];

pub fn is_recovery_tool(name: &str) -> bool {
    RECOVERY_TOOL_NAMES.contains(&name)
}

/// Whether large results are stored behind a handle (stats, head, handle)
/// instead of being compressed into one blob, and the REPL tools that query
/// them are offered to the model. Needs the router, the CCR store the handle
/// points into, and the compaction switch that lets the module rewrite results
/// at all.
pub fn repl_handle_active(config: &crate::config::Config) -> bool {
    config.context.compaction_enabled
        && config.tokenjuice.router_enabled
        && config.tokenjuice.ccr_enabled
        && config.tokenjuice.repl_handle_enabled
}

/// Whether TinyJuice may summarize this agent's tool output. Only the
/// orchestrator gets a summary model, and a zero threshold turns it off.
pub fn summarizes_tool_output(agent_id: &str, config: &crate::config::Config) -> bool {
    agent_id == "orchestrator" && config.context.summarizer_payload_threshold_tokens > 0
}

/// The TinyJuice tools a compacted result can point the model at: the CCR
/// recovery tool while anything can hand out a `⟦tj:…⟧` marker or a summary
/// footer, plus the REPL tools while results are stored behind a handle.
///
/// This is the single list both the session's visible-tool set and the harness
/// allowlist are built from. A name in the former but not the latter is
/// advertised in the prompt and the tool declarations yet answered as an
/// unknown tool at dispatch, so the model burns its failure budget on a tool it
/// was told to call.
pub fn companion_tool_names(agent_id: &str, config: &crate::config::Config) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    if config.context.compaction_enabled || summarizes_tool_output(agent_id, config) {
        names.extend(RECOVERY_TOOL_VISIBLE.iter().copied());
    }
    if repl_handle_active(config) {
        names.extend(REPL_TOOL_NAMES.iter().copied());
    }
    names
}

/// Where the module writes a plain-text copy of each stored original when
/// `[tokenjuice] repl_save_enabled` is on.
pub fn repl_save_dir(workspace_dir: &std::path::Path) -> std::path::PathBuf {
    workspace_dir.join(".tokenjuice").join("repl")
}

/// What the module is told to do, from the resolved configuration.
pub(crate) fn install_request(config: &crate::config::Config) -> InstallRequest {
    let tj = &config.tokenjuice;
    InstallRequest {
        options: types::CompressOptions {
            router_enabled: tj.router_enabled,
            ccr_enabled: tj.ccr_enabled,
            search_enabled: tj.search_enabled,
            code_enabled: tj.code_enabled,
            html_enabled: tj.html_enabled,
            min_bytes_to_compress: tj.min_bytes_to_compress,
            ccr_min_tokens: tj.ccr_min_tokens,
            // The summary stage still needs a context token per call, which
            // only a turn whose agent carries a summary model supplies — so
            // switching it on here enables it for those turns, not for all.
            llm_summary_enabled: config.context.summarizer_payload_threshold_tokens > 0,
            // Never at ingest: a large result gets deterministic compression
            // and a recovery handle, and the model writes a summary only when
            // the agent asks for one with `juice_summarize` (#6955).
            llm_summary_mode: types::LlmSummaryMode::OnDemand,
            llm_summary_threshold_tokens: config.context.summarizer_payload_threshold_tokens,
            llm_summary_max_input_tokens: config.context.summarizer_max_payload_tokens,
            repl_handle: repl_handle_active(config),
            repl_save_dir: (repl_handle_active(config) && tj.repl_save_enabled)
                .then(|| repl_save_dir(&config.workspace_dir)),
            ..types::CompressOptions::default()
        },
        max_cache_entries: tj.max_cache_entries,
        max_cache_bytes: tj.max_cache_bytes,
        ccr_ttl_secs: tj.ccr_ttl_secs,
        disk_tier_root: tj
            .ccr_disk_enabled
            .then(|| config.workspace_dir.join(".tokenjuice").join("ccr"))
            .map(|path| path.to_string_lossy().into_owned()),
    }
}

pub async fn install_from_config(config: &crate::config::Config) -> Result<(), String> {
    savings::configure(
        config
            .default_model
            .clone()
            .unwrap_or_else(|| crate::config::DEFAULT_MODEL.to_string()),
        &config.workspace_dir,
    );
    let request = install_request(config);
    let fingerprint = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    static INSTALLED: std::sync::OnceLock<tokio::sync::Mutex<Option<Vec<u8>>>> =
        std::sync::OnceLock::new();
    let mut installed = INSTALLED
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    if installed.as_ref() == Some(&fingerprint) {
        return Ok(());
    }
    client(config)
        .call::<()>("tinyjuice", methods::INSTALL, (request,))
        .await
        .map_err(|e| e.to_string())?;
    *installed = Some(fingerprint);
    Ok(())
}

/// Construct explicit access over the shared process-wide loader.
fn client(config: &crate::config::Config) -> crate::modules::client::ModuleClient {
    #[allow(unused_mut)] // Fixture overrides are compiled only into tests.
    let mut config = config.clone();
    #[cfg(test)]
    if let Some(path) = std::env::var_os("TINYJUICE_TEST_MODULE") {
        config
            .modules
            .overrides
            .push(crate::config::schema::ModuleOverride {
                id: "tinyjuice".into(),
                path: path.to_string_lossy().into_owned(),
            });
    }
    crate::modules::client::ModuleClient::new(config)
}

/// Everything the module considers about one tool result.
pub struct ToolOutputCompaction<'a> {
    pub content: String,
    pub tool_name: &'a str,
    pub enabled: bool,
    pub profile: AgentTokenjuiceCompression,
    pub runtime_config: Option<&'a std::sync::Arc<crate::config::Config>>,
    /// The tool call's JSON arguments, for command, extension and query hints.
    pub arguments: Option<serde_json::Value>,
    /// What the calling model said it needs from this result.
    pub focus: Option<String>,
    /// A [`generate::GenerateTicket`] token. `None` keeps the module from
    /// asking for a summary.
    pub context_token: Option<String>,
    /// Scopes summary reuse and the failure breaker, usually the thread id.
    pub scope: Option<String>,
}

/// What came back for one tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactedToolOutput {
    pub text: String,
    /// Prefix after the host's caps: the summary stage ran and wrote nothing.
    pub notice: Option<String>,
    /// Set when `text` is a model-written summary of this many bytes.
    pub summarized_from_bytes: Option<usize>,
}

impl CompactedToolOutput {
    fn unchanged(text: String) -> Self {
        Self {
            text,
            notice: None,
            summarized_from_bytes: None,
        }
    }

    /// The content untouched, disclosing the summary that did not happen when
    /// one was wanted.
    fn passthrough(text: String, wants_summary: bool) -> Self {
        Self {
            notice: wants_summary.then(summary_failed_notice),
            ..Self::unchanged(text)
        }
    }
}

/// TinyJuice's own notice for a summary that was attempted and not produced,
/// so a host-side failure reads the same as a module-side one.
pub fn summary_failed_notice() -> String {
    tinyjuice_bus::summary::UnavailableReason::Failed
        .notice()
        .to_string()
}

/// Deadline for `CompactWith`. It may wait on a summary model call, which the
/// module itself bounds at 300s; this sits just past that so the module's own
/// timeout is the one that fires.
const COMPACT_WITH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(330);

/// Compact one tool result through the module, passing it through unchanged on
/// any failure.
pub async fn compact_tool_output(call: ToolOutputCompaction<'_>) -> CompactedToolOutput {
    let ToolOutputCompaction {
        content,
        tool_name,
        enabled,
        profile,
        runtime_config,
        arguments,
        focus,
        context_token,
        scope,
    } = call;
    // A summary call is registered, so the result qualified for one. Any path
    // below that ends without a module answer says so, rather than handing
    // the model a raw dump it will re-run the tool to get summarized.
    let wants_summary = context_token.is_some();
    // This agent bypasses TinyJuice's router entirely. The middleware caller
    // never registers a summary ticket for an Off profile, but if one somehow
    // is, disclose it the same way every other no-answer path does rather
    // than silently dropping it in an `unchanged` return.
    if profile == AgentTokenjuiceCompression::Off {
        return CompactedToolOutput::passthrough(content, wants_summary);
    }
    // Nothing to ask the module for: the router is off and no summary call is
    // registered.
    if !enabled && !wants_summary {
        return CompactedToolOutput::unchanged(content);
    }
    #[cfg(test)]
    if let Ok(stub) = module_stub::STUB.try_with(std::sync::Arc::clone) {
        let response = stub(types::CompactRequest {
            content,
            tool_name: tool_name.to_string(),
            enabled,
            profile,
            arguments,
            focus,
            context_token,
            scope,
        })
        .await;
        return compacted_from(response);
    }
    let config = match runtime_config {
        Some(config) => std::sync::Arc::clone(config),
        None => match crate::config::Config::load_or_init().await {
            Ok(config) => std::sync::Arc::new(config),
            Err(error) => {
                log::debug!("[tokenjuice] config unavailable, passing through: {error}");
                return CompactedToolOutput::passthrough(content, wants_summary);
            }
        },
    };
    #[cfg(test)]
    let config = if std::env::var_os("TINYJUICE_TEST_MODULE").is_some() {
        // The released-module regression must not inherit an operator's
        // persisted compression thresholds or disabled router flags.
        std::sync::Arc::new(crate::config::Config::default())
    } else {
        config
    };
    if let Err(error) = install_from_config(&config).await {
        log::debug!("[tokenjuice] module configuration failed, passing through: {error}");
        return CompactedToolOutput::passthrough(content, wants_summary);
    }
    let request = types::CompactRequest {
        content: content.clone(),
        tool_name: tool_name.to_string(),
        enabled,
        profile,
        arguments,
        focus,
        context_token,
        scope,
    };
    let response = match client(&config)
        .with_timeout(COMPACT_WITH_TIMEOUT)
        .call::<types::CompactResponse>("tinyjuice", methods::COMPACT_WITH, (request,))
        .await
    {
        Ok(response) => response,
        Err(error) => {
            log::debug!("[tokenjuice] module compaction unavailable: {error}");
            return CompactedToolOutput::passthrough(content, wants_summary);
        }
    };
    record_savings(&response);
    compacted_from(response)
}

fn compacted_from(response: types::CompactResponse) -> CompactedToolOutput {
    let summarized_from_bytes = (response.compressor == CompressorKind::LlmSummary.as_str())
        .then_some(response.original_bytes);
    CompactedToolOutput {
        text: response.text,
        notice: response.notice,
        summarized_from_bytes,
    }
}

fn record_savings(response: &types::CompactResponse) {
    use std::str::FromStr as _;
    let kind = ContentKind::from_str(&response.content_kind).unwrap_or(ContentKind::PlainText);
    let compressor = CompressorKind::from_str(&response.compressor).unwrap_or(CompressorKind::None);
    savings::record(
        kind,
        compressor,
        response.original_tokens,
        response.compacted_tokens,
    );
}

pub async fn detect(content: String, hint: types::ContentHint) -> Result<String, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    client(&config)
        .call("tinyjuice", methods::DETECT, (content, hint))
        .await
        .map_err(|error| error.to_string())
}

pub async fn compress(
    content: String,
    hint: types::ContentHint,
) -> Result<types::CompressedOutput, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    install_from_config(&config).await?;
    let response: types::CompressedOutput = client(&config)
        .call("tinyjuice", methods::COMPRESS, (content, hint))
        .await
        .map_err(|error| error.to_string())?;
    savings::record(
        response.content_kind,
        response.compressor,
        (response.original_bytes as u64).div_ceil(4),
        (response.compacted_bytes as u64).div_ceil(4),
    );
    Ok(response)
}

pub async fn retrieve(
    token: String,
    range: Option<types::RetrieveRange>,
) -> Result<Option<String>, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    install_from_config(&config).await?;
    client(&config)
        .call("tinyjuice", methods::RETRIEVE, (token, range))
        .await
        .map_err(|error| error.to_string())
}

pub async fn cache_stats() -> Result<types::CacheStats, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    install_from_config(&config).await?;
    client(&config)
        .call("tinyjuice", methods::CACHE_STATS, ())
        .await
        .map_err(|error| error.to_string())
}

pub fn all_tokenjuice_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    schemas::all_registered_controllers()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "module_stub_tests.rs"]
pub(crate) mod module_stub;

#[cfg(test)]
#[path = "mod_repl_module_tests.rs"]
mod repl_module_tests;

/// Run a typed query inside TinyJuice; cached originals stay inside the module.
pub async fn query(
    request: tinyjuice_bus::wire::QueryRequest,
) -> Result<tinyjuice_bus::wire::QueryResponse, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|e| e.to_string())?;
    query_for_config(&config, request).await
}

pub(crate) async fn query_for_config(
    config: &crate::config::Config,
    request: tinyjuice_bus::wire::QueryRequest,
) -> Result<tinyjuice_bus::wire::QueryResponse, String> {
    install_from_config(config).await?;
    client(config)
        .call("tinyjuice", methods::QUERY, (request,))
        .await
        .map_err(|e| e.to_string())
}

/// Extract HTML through the compiled module independently of compression settings.
pub async fn extract_html(html: String) -> Result<String, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|e| e.to_string())?;
    let response: tinyjuice_bus::wire::HtmlResponse = client(&config)
        .call("tinyjuice", methods::EXTRACT_HTML, (html,))
        .await
        .map_err(|e| e.to_string())?;
    response.map_err(|_| "juice: HTML input exceeds module limit".into())
}
