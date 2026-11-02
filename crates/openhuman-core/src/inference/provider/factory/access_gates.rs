//! Access chokepoints every provider constructor runs through: privacy-mode
//! `LocalOnly` enforcement, the backend-session requirement, and egress disclosure.

use super::{BYOK_INCOMPLETE_SENTINEL, CLAUDE_AGENT_SDK_PROVIDER, PROVIDER_OPENHUMAN};
use crate::config::Config;
use crate::credentials::AuthService;
use crate::security::egress::EgressLedger;
use tinyinference_providers::route::{ProviderRoute, RouteKind};

/// Human-readable label for an *external* provider string, used in the
/// LocalOnly privacy-mode block message so the user knows what was refused.
pub(super) fn external_provider_label(provider: &str) -> String {
    let p = provider.trim();
    if p == PROVIDER_OPENHUMAN {
        return "OpenHuman (managed cloud)".to_string();
    }
    if p == BYOK_INCOMPLETE_SENTINEL {
        return "cloud (incomplete BYOK config)".to_string();
    }
    if p == CLAUDE_AGENT_SDK_PROVIDER
        || tinyinference_providers::route::ProviderRoute::parse(p)
            .is_some_and(|route| matches!(route.kind(), RouteKind::ClaudeAgentSdk))
    {
        return "Claude Agent SDK".to_string();
    }
    if p.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX) {
        return "Claude Code CLI".to_string();
    }
    // A concrete external route exposes just its provider slug.
    if let Some(route) = ProviderRoute::parse(p) {
        if route.is_external() {
            if let RouteKind::Cloud { slug } = route.kind() {
                if !slug.trim().is_empty() {
                    return slug.trim().to_string();
                }
            }
        }
    }
    p.to_string()
}

/// Privacy Mode (#4435) pure decision: under `mode`, is constructing chat
/// provider `provider` a local-only violation? Returns `Some(label)` naming the
/// blocked external provider when refused, else `None`.
///
/// Only `LocalOnly` restricts anything. Local runtimes (Ollama / LM Studio / MLX
/// / local-openai) are always permitted. Re-resolving sentinels (`""` / `"cloud"`)
/// return `None` here — they are resolved before model construction and
/// re-checked with the concrete
/// resolved string. Extracted as a pure fn so it is unit-testable without the
/// process-global live policy.
pub(super) fn local_only_violation(
    mode: crate::config::PrivacyMode,
    provider: &str,
) -> Option<String> {
    use crate::config::PrivacyMode;
    if mode != PrivacyMode::LocalOnly {
        return None;
    }
    let p = provider.trim();
    if p.is_empty() || p == "cloud" {
        // Deferred: re-resolves to a concrete string on the recursive call.
        return None;
    }
    if tinyinference_local::profile::is_local_provider_string(p) && !is_ollama_cloud_model(p) {
        return None;
    }
    Some(external_provider_label(p))
}

/// An Ollama `*-cloud` tag: the local daemon proxies it to ollama.com, so
/// the prompt leaves the machine although the route is named local.
pub(super) fn is_ollama_cloud_model(provider: &str) -> bool {
    let Some((prefix, model)) = provider.trim().split_once(':') else {
        return false;
    };
    let tag = model
        .rsplit_once('@')
        .map_or(model, |(tag, _temperature)| tag);
    prefix.eq_ignore_ascii_case("ollama") && tag.to_ascii_lowercase().ends_with("-cloud")
}

/// Whether `endpoint` stays on this machine. Only loopback counts: a LAN
/// address is another machine.
pub(super) fn is_loopback_endpoint(endpoint: &str) -> bool {
    match url::Url::parse(endpoint.trim())
        .ok()
        .and_then(|url| url.host().map(|h| h.to_owned()))
    {
        Some(url::Host::Domain(host)) => {
            let host = host.to_ascii_lowercase();
            host == "localhost" || host.ends_with(".localhost")
        }
        // A client dialling the unspecified address reaches this machine.
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        None => false,
    }
}

/// Privacy at request time for every local runtime. The construction-time
/// checks see only the route as built, and two things can change after that:
/// - the privacy mode is live, so a model built under `Standard` must still
///   stop at a non-loopback endpoint once the mode is local-only;
/// - a caller may override the route's model per request (a flow node's pinned
///   model, for one), so an Ollama route built for `llama3` can still be asked
///   for a `*-cloud` tag, which local-only refuses and every other mode
///   discloses as external egress.
pub(super) struct LocalOnlyRequestGuard {
    pub(super) inner: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    pub(super) role: String,
    /// The runtime's slug (`ollama`, `lmstudio`, ...), resolved endpoint, and
    /// the model the route was built (and disclosed) for.
    pub(super) slug: String,
    pub(super) endpoint: String,
    pub(super) model: String,
    /// The turn's ledger, so a per-request model is disclosed once per turn.
    pub(super) egress: EgressLedger,
}

impl LocalOnlyRequestGuard {
    fn check(
        &self,
        request: &tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<()> {
        let refuse = |e: anyhow::Error| tinyinference_llm::Error::Validation(e.to_string());
        enforce_local_only_endpoint(&self.role, &self.endpoint).map_err(refuse)?;
        let model = request.model.as_deref().unwrap_or(&self.model);
        let route = format!("{}:{model}", self.slug);
        let cloud = is_ollama_cloud_model(&route);
        if cloud {
            enforce_local_only_inference(&self.role, &route).map_err(refuse)?;
        }
        if model != self.model {
            // Construction disclosed the route's own model; an override is a
            // different destination, disclosed where the request makes it.
            crate::security::egress::emit_external_transfer_in(
                &self.egress,
                crate::security::egress::EgressDescriptor::inference(
                    self.slug.clone(),
                    model.to_string(),
                    cloud || !is_loopback_endpoint(&self.endpoint),
                ),
            );
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl tinyinference_llm::model::ChatModel<()> for LocalOnlyRequestGuard {
    fn profile(&self) -> Option<&tinyinference_llm::model::ModelProfile> {
        self.inner.profile()
    }
    fn supports_input(
        &self,
        modality: tinyinference_llm::model::InputModality,
        mime_type: &str,
        source: tinyinference_llm::model::InputSource,
    ) -> bool {
        self.inner.supports_input(modality, mime_type, source)
    }
    fn cache_identity(&self) -> Option<String> {
        self.inner.cache_identity()
    }
    fn tracks_physical_attempts(&self) -> bool {
        self.inner.tracks_physical_attempts()
    }
    fn has_pre_invocation_admission_gate(&self) -> bool {
        self.inner.has_pre_invocation_admission_gate()
    }
    async fn invoke(
        &self,
        state: &(),
        request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelResponse> {
        self.check(&request)?;
        self.inner.invoke(state, request).await
    }
    async fn stream(
        &self,
        state: &(),
        request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelStream> {
        self.check(&request)?;
        self.inner.stream(state, request).await
    }
    async fn fetch_deferred(
        &self,
        handle: &tinyinference_llm::model::DeferredHandle,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::DeferredStatus> {
        self.inner.fetch_deferred(handle).await
    }
}

/// Privacy Mode `LocalOnly` for a resolved local-runtime endpoint: refuse
/// one that is not loopback (the prompt and the `local_ai` key would go there).
pub(super) fn enforce_local_only_endpoint(role: &str, endpoint: &str) -> anyhow::Result<()> {
    let mode = crate::security::live_policy::current_privacy_mode();
    if mode != crate::config::PrivacyMode::LocalOnly || is_loopback_endpoint(endpoint) {
        return Ok(());
    }
    log::warn!("[privacy][chat-factory] LocalOnly BLOCK: role={role} local runtime endpoint is not loopback");
    anyhow::bail!(
        "Local-only privacy mode is active: this local model's endpoint is on another machine. \
         Point it at this computer (localhost) or change privacy mode in Settings."
    )
}

/// Enforce Privacy Mode `LocalOnly` at the inference chokepoint: refuse to build
/// an external chat provider when the live policy is local-only. Reads the live
/// privacy mode (defaults to `Standard`/allow when no session policy is
/// installed). See [`local_only_violation`] for the pure decision.
pub(super) fn enforce_local_only_inference(role: &str, provider: &str) -> anyhow::Result<()> {
    let mode = crate::security::live_policy::current_privacy_mode();
    match local_only_violation(mode, provider) {
        None => {
            log::debug!(
                "[privacy][chat-factory] privacy_mode={:?} role={} provider='{}' — inference permitted",
                mode,
                role,
                provider.trim()
            );
            Ok(())
        }
        Some(label) => {
            log::warn!(
                "[privacy][chat-factory] LocalOnly BLOCK: role={} external provider='{}' ({}) refused",
                role,
                provider.trim(),
                label
            );
            anyhow::bail!(
                "Local-only privacy mode is active: this action needs external provider {label}. \
                 Switch to a local model (Ollama/LM Studio/etc.) or change privacy mode in Settings."
            )
        }
    }
}

/// Egress spine (privacy epic S2, #4436): emit an [`EgressDescriptor`] for a
/// concrete inference provider string. `provider` is expected to be already
/// resolved (no `""` / `"cloud"` / BYOK sentinels — those are handled before
/// this is called). Local runtimes are marked non-external, so
/// [`emit_external_transfer`](crate::security::egress::emit_external_transfer)
/// discloses them without firing the external-transfer event.
pub(super) fn emit_inference_egress(role: &str, provider: &str, egress: &EgressLedger) {
    let p = provider.trim();
    if p.is_empty() || p == "cloud" {
        // Defensive: a sentinel would re-resolve on recursion; don't emit here.
        return;
    }
    if p == PROVIDER_OPENHUMAN {
        // Managed backend is emitted centrally in `managed_chat_model`,
        // the universal managed ChatModel funnel. Skipping here avoids a
        // duplicate descriptor.
        return;
    }
    let is_local = tinyinference_local::profile::is_local_provider_string(p);
    let (slug, model) = match p.split_once(':') {
        Some((s, m)) if !s.trim().is_empty() => (s.trim().to_string(), m.trim().to_string()),
        _ => (p.to_string(), String::new()),
    };
    // Fall back to the workload role when the provider string carries no model
    // component (e.g. a bare `"openhuman"` / `"ollama"` slug).
    let service = if model.is_empty() {
        role.to_string()
    } else {
        model
    };
    crate::security::egress::emit_external_transfer_in(
        egress,
        crate::security::egress::EgressDescriptor::inference(slug, service, !is_local),
    );
}

/// Caller-owned runtimes authenticate independently of the OpenHuman account.
/// Claude subprocesses still use external inference and remain subject to the
/// privacy gate; this exemption only concerns OpenHuman registration.
pub(crate) fn provider_uses_independent_auth(provider: &str) -> bool {
    let p = provider.trim();
    tinyinference_local::profile::is_local_provider_string(p)
        || p == CLAUDE_AGENT_SDK_PROVIDER
        || tinyinference_providers::route::ProviderRoute::parse(p).is_some_and(|route| {
            matches!(
                route.kind(),
                RouteKind::Local(_) | RouteKind::ClaudeCode | RouteKind::ClaudeAgentSdk
            )
        })
}

pub(crate) async fn verify_provider_session(config: &Config, provider: &str) -> anyhow::Result<()> {
    if provider_uses_independent_auth(provider) {
        log::debug!("[chat-factory] caller-owned runtime uses independent authentication");
        return Ok(());
    }
    verify_session_active(config).await
}

/// Explicit-dispatch counterpart used by model construction. Session policy
/// follows the host context that requested the model, never a different ambient
/// context on the current task.
pub(crate) async fn verify_provider_session_in(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    provider: &str,
) -> anyhow::Result<()> {
    if provider_uses_independent_auth(provider) {
        log::debug!("[chat-factory] caller-owned runtime uses independent authentication");
        return Ok(());
    }
    let requires_session = ctx
        .resolved()
        .map(|core| host_requires_session(core.host_kind()))
        .unwrap_or(true);
    verify_session_active_for_host(config, requires_session).await
}

/// Verify the user has an active OpenHuman backend session.
///
/// Without this check, an unregistered user can configure every workload
/// to use a custom cloud provider and bypass the session requirement
/// entirely. Custom cloud routes require an `app-session` JWT; caller-owned
/// local runtimes and Claude subprocesses use `verify_provider_session` instead.
///
/// `pub(crate)`: also reused directly by the flows provider-connectivity
/// author gate (issue B45, `openhuman::flows::ops::evaluate_inference_readiness`)
/// as its Layer 1 sync session check, so the author-time gate and this
/// construction-time chokepoint can never diverge on what "session active"
/// means.
pub(crate) async fn verify_session_active(config: &Config) -> anyhow::Result<()> {
    // An in-process library host is given its provider URL and credential by
    // its caller. It does not participate in the OpenHuman app's registration
    // or auth-profile lifecycle, so requiring a fabricated local session here
    // would mix desktop product policy into the library contract. The ambient
    // context is installed by the dispatch chokepoint; all other host kinds
    // retain the registration gate below.
    verify_session_active_for_host(config, current_host_requires_session()).await
}

async fn verify_session_active_for_host(
    config: &Config,
    requires_session: bool,
) -> anyhow::Result<()> {
    if !requires_session {
        return Ok(());
    }
    // A hosted profile on the operator's direct chat provider talks to that
    // provider with the operator's key; it has no TinyHumans account.
    if config
        .profile_providers
        .as_ref()
        .is_some_and(|providers| providers.has_direct_chat())
    {
        return Ok(());
    }

    verify_backend_session_active(config).await
}

pub(super) fn verify_provider_session_in_with_snapshot(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    provider: &str,
    snapshot: &super::credentials::RuntimeCredentialSnapshot,
) -> anyhow::Result<()> {
    if provider_uses_independent_auth(provider) {
        return Ok(());
    }
    let requires_session = ctx
        .resolved()
        .map(|core| host_requires_session(core.host_kind()))
        .unwrap_or(true);
    verify_session_active_with_snapshot_for_host(config, requires_session, snapshot)
}

/// Synchronous model-factory gate backed by the per-session credential
/// snapshot loaded before TinyAgents invokes its sync `ModelFactory` trait.
pub(super) fn verify_session_active_with_snapshot(
    config: &Config,
    snapshot: &super::credentials::RuntimeCredentialSnapshot,
) -> anyhow::Result<()> {
    verify_session_active_with_snapshot_for_host(config, current_host_requires_session(), snapshot)
}

fn verify_session_active_with_snapshot_for_host(
    config: &Config,
    requires_session: bool,
    snapshot: &super::credentials::RuntimeCredentialSnapshot,
) -> anyhow::Result<()> {
    if !requires_session
        || config
            .profile_providers
            .as_ref()
            .is_some_and(|providers| providers.has_direct_chat())
    {
        return Ok(());
    }
    if snapshot.backend_session_active() {
        return Ok(());
    }
    anyhow::bail!("SESSION_EXPIRED: backend session not active — sign in to use custom providers")
}

/// Managed `OpenhumanJwt` inference always needs the backend bearer, including
/// in a Library host. Library mode exempts caller-owned provider credentials;
/// it cannot manufacture a TinyHumans account credential.
pub(crate) async fn verify_backend_session_active(config: &Config) -> anyhow::Result<()> {
    let state_dir = config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".openhuman"))
                .unwrap_or_else(|| std::path::PathBuf::from(".openhuman"))
        });
    // An API key is a standing credential: no session to be active, nothing
    // for the scheduler gate to have expired.
    if crate::credentials::api_key::has_api_key_in(&state_dir, config.secrets.encrypt).await {
        log::debug!(
            "[providers][access-gate] api-key credential satisfies the backend session gate"
        );
        return Ok(());
    }
    // Fast path: the scheduler gate already knows the session is dead.
    if crate::platform::llm_gate::is_signed_out() {
        anyhow::bail!(
            "SESSION_EXPIRED: backend session not active — sign in to use custom providers"
        );
    }
    // Verify the app-session JWT actually exists in auth-profiles.
    let auth = AuthService::new(&state_dir, config.secrets.encrypt);
    let has_session = auth
        .get_provider_bearer_token(crate::credentials::APP_SESSION_PROVIDER, None)
        .await?
        .filter(|s| !s.trim().is_empty())
        .is_some();
    if !has_session {
        anyhow::bail!("SESSION_EXPIRED: no backend session — sign in to use OpenHuman")
    }
    Ok(())
}

/// Whether inference in the ambient host participates in OpenHuman app login.
/// Library hosts receive their provider configuration from the embedding
/// process; every other host retains the product session policy.
pub(crate) fn current_host_requires_session() -> bool {
    crate::core::runtime::context::CoreContext::current()
        .map(|ctx| host_requires_session(ctx.host_kind()))
        .unwrap_or(true)
}

pub(super) fn host_requires_session(host_kind: crate::core::types::HostKind) -> bool {
    host_kind != crate::core::types::HostKind::Library
}

/// The access check every model a harness turn builds passes first.
impl tinyagents_harness::host::ModelBuildGate for super::OpenHumanModelFactory<'_> {
    fn check(
        &self,
        request: &tinyagents_harness::host::ModelBuildRequest,
    ) -> tinyagents_harness::Result<()> {
        // Explicit host model injection is authoritative and has no provider
        // construction, credential, or egress side effect to gate.
        if self.injected(request).is_some() {
            return Ok(());
        }
        let provider = self.provider_for(request);
        enforce_local_only_inference(&request.role, &provider)
            .map_err(|error| tinyagents_harness::TinyAgentsError::Model(error.to_string()))?;
        verify_provider_session_in_with_snapshot(
            self.ctx,
            self.config,
            &provider,
            &self.credentials,
        )
        .map_err(|error| tinyagents_harness::TinyAgentsError::Model(error.to_string()))
    }
}
