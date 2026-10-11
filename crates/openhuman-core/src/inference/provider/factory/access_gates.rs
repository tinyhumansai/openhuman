//! Access chokepoints every provider constructor runs through: privacy-mode
//! `LocalOnly` enforcement, the backend-session requirement, and egress disclosure.

use super::*;

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
    if p == CLAUDE_AGENT_SDK_PROVIDER || p.starts_with(CLAUDE_AGENT_SDK_PREFIX) {
        return "Claude Agent SDK".to_string();
    }
    if p.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX) {
        return "Claude Code CLI".to_string();
    }
    // Concrete cloud slug "<slug>:<model>" → surface just the slug.
    match p.split_once(':') {
        Some((slug, _)) if !slug.trim().is_empty() => slug.trim().to_string(),
        _ => p.to_string(),
    }
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
    if tinyinference_local::profile::is_local_provider_string(p) {
        return None;
    }
    Some(external_provider_label(p))
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
pub(super) fn emit_inference_egress(role: &str, provider: &str) {
    let p = provider.trim();
    if p.is_empty() || p == "cloud" {
        // Defensive: a sentinel would re-resolve on recursion; don't emit here.
        return;
    }
    if p == PROVIDER_OPENHUMAN {
        // Managed backend is emitted centrally in `resolve_managed_backend`,
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
    crate::security::egress::emit_external_transfer(
        crate::security::egress::EgressDescriptor::inference(slug, service, !is_local),
    );
}

/// Caller-owned runtimes authenticate independently of the OpenHuman account.
/// Claude subprocesses still use external inference and remain subject to the
/// privacy gate; this exemption only concerns OpenHuman registration.
pub(crate) fn provider_uses_independent_auth(provider: &str) -> bool {
    let p = provider.trim();
    tinyinference_local::profile::is_local_provider_string(p)
        || p.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
        || p == CLAUDE_AGENT_SDK_PROVIDER
        || p.starts_with(CLAUDE_AGENT_SDK_PREFIX)
}

pub(crate) fn verify_provider_session(config: &Config, provider: &str) -> anyhow::Result<()> {
    if provider_uses_independent_auth(provider) {
        log::debug!("[chat-factory] caller-owned runtime uses independent authentication");
        return Ok(());
    }
    verify_session_active(config)
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
pub(crate) fn verify_session_active(config: &Config) -> anyhow::Result<()> {
    // An in-process library host is given its provider URL and credential by
    // its caller. It does not participate in the OpenHuman app's registration
    // or auth-profile lifecycle, so requiring a fabricated local session here
    // would mix desktop product policy into the library contract. The ambient
    // context is installed by the dispatch chokepoint; all other host kinds
    // retain the registration gate below.
    if !current_host_requires_session() {
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

    verify_backend_session_active(config)
}

/// Managed `OpenhumanJwt` inference always needs the backend bearer, including
/// in a Library host. Library mode exempts caller-owned provider credentials;
/// it cannot manufacture a TinyHumans account credential.
pub(crate) fn verify_backend_session_active(config: &Config) -> anyhow::Result<()> {
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
    if crate::security::credentials::api_key::has_api_key_in(&state_dir, config.secrets.encrypt) {
        log::debug!(
            "[providers][access-gate] api-key credential satisfies the backend session gate"
        );
        return Ok(());
    }
    // Fast path: the scheduler gate already knows the session is dead.
    if crate::cron::scheduler_gate::is_signed_out() {
        anyhow::bail!(
            "SESSION_EXPIRED: backend session not active — sign in to use custom providers"
        );
    }
    // Verify the app-session JWT actually exists in auth-profiles.
    let auth = AuthService::new(&state_dir, config.secrets.encrypt);
    let has_session = auth
        .get_provider_bearer_token(crate::security::credentials::APP_SESSION_PROVIDER, None)?
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
