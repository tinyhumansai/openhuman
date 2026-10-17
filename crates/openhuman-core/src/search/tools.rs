//! Agent tools backed by the TinySearch module.
//!
//! Declarations come from `modules::search::configured_tool_specs`, which is
//! computed synchronously from config so a turn's tool list is stable and
//! needs no module round-trip. Each call re-reads the live config and goes
//! through `modules::search::execute_tool`, so a provider or login change is
//! honoured on the next call without rebuilding the session — unless the
//! agent's config carries host-pinned search keys, which make it the config
//! the call runs under.

use crate::core::runtime::DispatchCtx;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use tinysearch_bus::errors::Category;
use tinysearch_bus::{errors, ExecuteToolRequest, ToolSpec};
use tinytools::{Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolResult};

use crate::config::Config;

/// One TinySearch tool (a role tool such as `web_search_tool`, or a provider
/// tool in `all_tools` presentation).
/// What a call reports once every provider has already answered
/// "unavailable" in this session. It tells the model to stop rather than to
/// wait, because nothing about the session will change the answer (#6991).
pub const SEARCH_EXHAUSTED_MESSAGE: &str =
    "Web search is not available with this setup: no provider could answer. Do not call \
     it again \u{2014} answer from the material you already have.";

pub struct TinySearchTool {
    spec: ToolSpec,
    /// Spawn-time config. `None` for a deferred instance rebuilt from a
    /// recorded transcript, which resolves the live config per call.
    config: Option<Arc<Config>>,
    exposure: ToolExposure,
    /// The provider configuration a call last found nothing usable under.
    ///
    /// A credential alone makes the managed route look reachable
    /// (`providers::backend_credential_available`), so a deployment that is
    /// offline, firewalled, out of balance or holding a dead key offers this
    /// tool on every turn and fails every call. The agent then spends turns on
    /// a tool that cannot work, at the moment it is least sure what to do.
    ///
    /// Keyed by configuration rather than latched outright, because this
    /// module's contract is that every call re-reads the live config so a
    /// provider or login change is honoured without rebuilding the session. A
    /// call whose signature differs from the recorded one tries again; only a
    /// repeat under the same configuration is refused. The signature is this
    /// tool's own view — its role order and the resolved providers — so one
    /// tool's dead providers never answer for another's.
    exhausted_for: Mutex<Option<u64>>,
}

impl TinySearchTool {
    pub fn new(config: Arc<Config>, spec: ToolSpec) -> Self {
        Self {
            spec,
            config: Some(config),
            exposure: ToolExposure::Direct,
            exhausted_for: Mutex::new(None),
        }
    }

    /// A tool recorded in a resumed thread's transcript. It keeps the recorded
    /// declaration and resolves the live config on every call.
    pub fn recorded(spec: ToolSpec) -> Self {
        Self {
            spec,
            config: None,
            exposure: ToolExposure::Direct,
            exhausted_for: Mutex::new(None),
        }
    }

    /// Whether this tool already found nothing usable under `signature`.
    pub(crate) fn is_exhausted_for(&self, signature: u64) -> bool {
        self.exhausted_for
            .lock()
            .map(|recorded| *recorded == Some(signature))
            .unwrap_or(false)
    }

    /// Record that no provider could answer under `signature`. Replaces any
    /// earlier one, so the refusal always describes the current configuration.
    pub(crate) fn mark_exhausted_for(&self, signature: u64) {
        if let Ok(mut recorded) = self.exhausted_for.lock() {
            *recorded = Some(signature);
        }
    }

    /// What this tool's providers look like right now: the order its role
    /// draws from, and every provider's resolved reachability. Two calls agree
    /// only while nothing a user could change — a key, a route, a provider
    /// selection, a login — has moved.
    pub(crate) fn provider_signature(&self, config: &Config) -> u64 {
        let mut hasher = DefaultHasher::new();
        if let Some(role) = tinysearch_bus::role_for_tool(&self.spec.name) {
            for provider in super::providers::role_order(config, role) {
                provider.hash(&mut hasher);
            }
        }
        for provider in super::providers::resolve(config) {
            provider.id.hash(&mut hasher);
            provider.enabled.hash(&mut hasher);
            provider.usable.hash(&mut hasher);
            provider.key_configured.hash(&mut hasher);
            provider.managed_available.hash(&mut hasher);
            matches!(provider.route, crate::config::ProviderRoute::Backend).hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn live_config(&self, ctx: &DispatchCtx) -> anyhow::Result<Config> {
        match &self.config {
            Some(config) => Ok(config.as_ref().clone()),
            None => ctx
                .load_config()
                .await
                .map_err(|error| anyhow::anyhow!(error)),
        }
    }
}

/// Map a module error to a message the model can act on. The module prefixes
/// classified failures with `tinysearch.<code>:`; the detail after the prefix
/// may echo the query, so it is not logged.
pub fn user_facing_error(error: &str) -> String {
    let code = errors::code_of(error);
    match errors::classify(error) {
        Category::InsufficientBalance => {
            "Web search is unavailable: the TinyHumans balance is too low for managed search. \
             Top up the balance, or add your own provider key under Connections → Search."
                .to_string()
        }
        Category::RateLimited => {
            "Web search is rate limited right now. Wait a moment and try again.".to_string()
        }
        Category::Exhausted => SEARCH_EXHAUSTED_MESSAGE.to_string(),
        Category::Unauthorized if code == Some(errors::BACKEND_UNAUTHORIZED) => {
            backend_unauthorized_message(false)
        }
        Category::Unauthorized => {
            // The detail names the provider and its HTTP status; the module
            // never puts the key or the query in it.
            format!(
                "Web search is unavailable: {}. Update the key under Connections → Search, or \
                 remove it to use managed search.",
                code_detail(error, code.unwrap_or_default())
            )
        }
        Category::InvalidArguments => {
            format!(
                "The search request was rejected: {}",
                code_detail(error, code.unwrap_or_default())
            )
        }
        Category::Unclassified => format!("Web search failed: {error}"),
    }
}

/// The module's message after its `tinysearch.<code>: ` prefix.
fn code_detail<'a>(error: &'a str, code: &str) -> &'a str {
    let marker = format!("{}{code}: ", errors::PREFIX);
    error
        .split_once(marker.as_str())
        .map(|(_, detail)| detail)
        .unwrap_or(error)
}

/// What the model is told when the managed backend rejects the credential.
pub(crate) fn backend_unauthorized_message(api_key: bool) -> String {
    if api_key {
        "Web search is unavailable: the TinyHumans backend rejected the configured API key. \
         Check the key, or add your own provider key under Connections → Search."
            .to_string()
    } else {
        "Web search is unavailable: the OpenHuman sign-in has expired. Sign in again to \
         continue searching."
            .to_string()
    }
}

/// Recover from the managed backend rejecting the TinyHumans credential.
///
/// A rejected session is dead for every backend call, not just search, so
/// publish `SessionExpired`: the credentials subscriber clears the token and
/// the UI asks the user to sign in, the same recovery a backend 401 on
/// inference or integrations drives. Before this, a user on their own
/// inference key never saw that prompt and search kept failing on the stale
/// token. A rejected API key has no session to expire, so it never signs the
/// user out (`API_KEY_REJECTED` semantics).
fn on_backend_unauthorized(tool: &str, api_key: bool) {
    if api_key || crate::platform::llm_gate::is_signed_out() {
        return;
    }
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
        source: format!("search.{tool}"),
        reason: "managed search backend rejected the session (401)".to_string(),
    });
}

#[async_trait]
impl Tool for TinySearchTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters_schema(&self) -> Value {
        self.spec.parameters.clone()
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Workflow
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, options, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let ctx = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let config = self.live_config(&ctx).await?;
        // Re-read per call, so a key added or a provider switched mid-session
        // clears an earlier refusal instead of outliving it.
        let signature = self.provider_signature(&config);
        if self.is_exhausted_for(signature) {
            tracing::debug!(
                tool = %self.spec.name,
                "[search][tool] refused: no provider answered under this configuration"
            );
            return Ok(ToolResult::failed(SEARCH_EXHAUSTED_MESSAGE.to_string()));
        }
        let subject = super::render::subject(&args);
        // `limit` is the module's alias for `max_results`.
        let max_results = args
            .get("max_results")
            .or_else(|| args.get("limit"))
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(config.search.max_results)
            .clamp(1, 20);
        tracing::debug!(
            tool = %self.spec.name,
            subject_len = subject.chars().count(),
            "[search][tool] execute"
        );
        let request = ExecuteToolRequest {
            name: self.spec.name.clone(),
            arguments: args,
        };
        if let Some(message) = local_only_search_block(&request.name) {
            return Ok(ToolResult::error(message));
        }
        match crate::modules::search::execute_tool(&ctx, &config, request).await {
            Ok(response) => {
                tracing::debug!(
                    tool = %self.spec.name,
                    provider = %response.provider,
                    fallbacks = response.fallback_from.len(),
                    results = response.results.len(),
                    "[search][tool] completed"
                );
                Ok(super::render::render(
                    &response,
                    &subject,
                    max_results,
                    options.prefer_markdown,
                ))
            }
            Err(error) => {
                if errors::exhausts_providers(&error) {
                    self.mark_exhausted_for(signature);
                }
                if errors::code_of(&error) == Some(errors::BACKEND_UNAUTHORIZED) {
                    let api_key = crate::security::credentials::api_key::has_api_key(&config);
                    on_backend_unauthorized(&self.spec.name, api_key);
                    tracing::warn!(
                        tool = %self.spec.name,
                        api_key,
                        "[search][tool] managed backend rejected the TinyHumans credential"
                    );
                    return Ok(ToolResult::error(backend_unauthorized_message(api_key)));
                }
                tracing::warn!(
                    tool = %self.spec.name,
                    code = errors::code_of(&error).unwrap_or("unclassified"),
                    exhausted = self.is_exhausted_for(signature),
                    "[search][tool] failed"
                );
                Ok(if errors::exhausts_providers(&error) {
                    ToolResult::failed(user_facing_error(&error))
                } else {
                    ToolResult::error(user_facing_error(&error))
                })
            }
        }
    }
}

pub(crate) fn local_only_search_block(tool_name: &str) -> Option<String> {
    crate::security::egress::local_only_tool_block(&crate::security::egress::EgressDescriptor::new(
        "tinysearch",
        tool_name,
        true,
        crate::security::egress::EgressReason::ToolCall,
        vec![crate::security::egress::DataKind::ToolArguments],
    ))
}

/// Build the agent's search tools from config. Empty when search is off or no
/// provider is usable.
pub fn build_search_tools(config: &Config) -> Vec<Box<dyn Tool>> {
    let specs = crate::modules::search::configured_tool_specs(config);
    tracing::debug!(
        tools = ?specs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        "[search][tool] registered search tools"
    );
    let shared = Arc::new(config.clone());
    specs
        .into_iter()
        .map(|spec| Box::new(TinySearchTool::new(shared.clone(), spec)) as Box<dyn Tool>)
        .collect()
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
