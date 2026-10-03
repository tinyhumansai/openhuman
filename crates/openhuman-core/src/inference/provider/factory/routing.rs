//! Role → configured provider-string resolution (`provider_for_role`) plus the
//! managed-credits bypass check and the `<model>[@<temp>]` suffix parser.

use super::*;
use crate::inference::provider::fallback_diagnostics;

/// The provider route a role has **explicitly** configured, before any
/// fallback.
///
/// Split out of [`provider_for_role`] so the fallback machinery can ask the
/// same question the router asks — "did the user route this role anywhere?" —
/// without re-deriving the role→config-field mapping and drifting from it.
pub(super) fn configured_route_for_role<'a>(role: &str, config: &'a Config) -> Option<&'a str> {
    match role {
        "chat" => config.chat_provider.as_deref(),
        "reasoning" => config.reasoning_provider.as_deref(),
        "agentic" => config.agentic_provider.as_deref(),
        "coding" => config.coding_provider.as_deref(),
        // Burst uses the existing Agentic workload route for BYOK/local parity.
        // If unset, it falls through to the managed backend and is pinned to
        // `hint:burst` by `managed_tier_for_role`.
        "burst" => config.agentic_provider.as_deref(),
        // Tier-specific multimodal model; when unset it falls through to
        // `primary_cloud` (→ managed `hint:vision`), as every unset route now does.
        "vision" => config.vision_provider.as_deref(),
        // `memory_provider` routes the summarizer sub-agent (whose definition
        // declares `hint = "summarization"`): "produce a condensed
        // representation of input text".
        "memory" | "summarization" => config.memory_provider.as_deref(),
        "embeddings" => config.embeddings_provider.as_deref(),
        _ => None,
    }
}

/// Whether `role` reached a cloud slug by *implicit fallback* rather than by an
/// explicit route.
///
/// True only when the role is one of the cloud-fallback background roles **and**
/// its own route is unset (or the literal `"cloud"`). An explicitly configured
/// cloud route — say `vision_provider = "anthropic:claude-…"` — is not a
/// fallback, so a credential failure there must not be explained as "your local
/// chat model cannot do this".
pub(crate) fn role_uses_implicit_cloud_fallback(role: &str, config: &Config) -> bool {
    if !fallback_diagnostics::role_falls_back_to_cloud(role) {
        return false;
    }
    let route = configured_route_for_role(role, config).unwrap_or("").trim();
    route.is_empty() || route == "cloud"
}

/// Return the configured provider string for a named workload role.
///
/// Empty / `"cloud"` resolves through `primary_cloud`. **Every** role behaves
/// this way, and no role uses a sibling's BYOK provider as a fallback.
///
/// "Resolves through `primary_cloud`" is not the same as "goes to the managed
/// backend": [`resolve_primary_cloud_provider_string`] may yield a legacy
/// external `inference_url` (see below), or the fail-closed sentinel for a
/// half-migrated BYOK config. The guarantee here is narrower and exact — an
/// unset route does not borrow a *sibling's* configured provider.
///
/// The deliberate *aliases* in [`configured_route_for_role`] are unaffected and
/// remain: `burst` reads `agentic_provider` and `summarization` reads
/// `memory_provider`. Those are two names for one configured route, which is a
/// different thing from an unset route borrowing a set one.
///
/// Until #6109 the three chat-tier roles (`chat`, `reasoning`, `coding`) took a
/// configured BYOK provider from *any* sibling first, so setting one route moved
/// the other two onto that key — a user who pointed only `coding_provider` at
/// their own OpenRouter account found ordinary chat billed there too, with no
/// setting saying so. Each route now stands alone.
///
/// For backwards compatibility, a legacy external `inference_url` takes
/// precedence when `primary_cloud` still points at OpenHuman because
/// migration 1→2 preserved the URL as a custom provider entry but older
/// configs did not explicitly set per-workload routes.
pub fn provider_for_role(role: &str, config: &Config) -> String {
    let opt = configured_route_for_role(role, config);
    let s = opt.unwrap_or("").trim();
    if s.is_empty() || s == "cloud" {
        // #6109: an unset chat-tier route no longer inherits a sibling's BYOK
        // provider. Setting only `coding_provider` used to move `chat` and
        // `reasoning` onto that key too — ordinary conversations silently billed
        // to the user's own account, with no settings field saying so. Each
        // route now stands alone and an unset one falls through to the managed
        // backend, the same as every other workload.

        let resolved = resolve_primary_cloud_provider_string(config);

        // #5146 §2.1: the fallback itself is correct and stays — background
        // workloads run tier-specific models that local runtimes don't serve,
        // and a local-chat + managed-subscription user genuinely wants them on
        // the cloud. What was missing is the *explanation*: when this route
        // later fails for want of a key, the user saw a bare slug-level auth
        // error naming a provider they never configured. Emit the same
        // user-facing sentence the error path uses, so the routing decision is
        // visible in logs and support transcripts before anything goes wrong.
        if fallback_diagnostics::role_falls_back_to_cloud(role) {
            if let Some(chat) = config.chat_provider.as_deref() {
                if tinyinference_local::profile::is_local_provider_string(chat) {
                    log::info!(
                        "[providers][local-fallback] role={} {}",
                        role,
                        fallback_diagnostics::cloud_fallback_notice(role, chat, &resolved)
                    );
                }
            }
        }

        resolved
    } else {
        s.to_string()
    }
}

/// #3767: Whether the OpenHuman managed-credits gate should be bypassed for a
/// single workload role.
///
/// Returns true when `role` resolves (via [`provider_for_role`]) to a non-managed
/// provider the user funds themselves — a BYO cloud key (incl. OpenAI OAuth), a
/// local runtime, or claude-code — with usable credentials. When the role is on
/// the OpenHuman managed backend, or a BYO route has no usable key, it returns
/// false (the gate stays on; #3767: "BYO key present but invalid/unverified →
/// still gated").
///
/// The gate is evaluated per-tier so the UI can check the tier the user actually
/// selected: the chat header's "Quick" mode runs on the `chat` tier and
/// "Reasoning" mode on the `reasoning` tier, so each is checked respectively.
/// These per-role results are surfaced under `credits_bypass` in the
/// client-config snapshot. Tiers that stay managed and run anyway surface the
/// per-call `USER_INSUFFICIENT_CREDITS` (402) error reactively.
pub fn role_bypasses_managed_credits(role: &str, config: &Config) -> bool {
    let resolved = provider_for_role(role, config);
    let r = resolved.trim();
    let is_managed =
        r.is_empty() || r == "cloud" || r == PROVIDER_OPENHUMAN || r == BYOK_INCOMPLETE_SENTINEL;
    let usable_byo = !is_managed && route_has_usable_credentials(r, config);
    log::debug!(
        "[billing] role_bypasses_managed_credits role={role} resolved={resolved} \
         is_managed={is_managed} usable_byo={usable_byo}"
    );
    usable_byo
}

/// True when a resolved chat-tier provider string can actually run on the
/// user's own funding: local runtimes / claude-code carry their own creds; a
/// concrete cloud slug requires a non-empty stored key. Managed/sentinel
/// strings are filtered by the caller and never reach here as "usable".
pub(super) fn route_has_usable_credentials(resolved: &str, config: &Config) -> bool {
    let r = resolved.trim();
    // Local runtimes (ollama/lmstudio/mlx/local-openai) and the local CLI
    // delegates carry their own credentials / run on-device.
    if tinyinference_local::profile::is_local_provider_string(r)
        || r.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
        || r == CLAUDE_AGENT_SDK_PROVIDER
        || r.starts_with(CLAUDE_AGENT_SDK_PREFIX)
    {
        return true;
    }
    // Concrete cloud slug "<slug>:<model>" — require a usable stored key.
    if let Some((slug, _)) = r.split_once(':') {
        let slug = slug.trim();
        if !slug.is_empty() {
            // Don't silently swallow auth-store / OAuth lookup failures — a
            // transient Err would otherwise keep the credits gate on for a
            // valid BYO setup with no diagnostics. Log and treat as not-usable.
            match lookup_key_for_slug(slug, config) {
                Ok(key) => {
                    let usable = !key.trim().is_empty();
                    log::debug!(
                        "[billing] route_has_usable_credentials slug={slug} usable={usable}"
                    );
                    return usable;
                }
                Err(e) => {
                    log::debug!(
                        "[billing] route_has_usable_credentials slug={slug} lookup_error={e}"
                    );
                    return false;
                }
            }
        }
    }
    false
}

/// Parse a `<model>[@<temp>]` tail into `(model, override)`.
///
/// Tolerates whitespace around the components. Returns `temperature = None`
/// when the suffix is absent or unparseable — the model text is taken as-is.
pub(super) fn split_model_and_temperature(raw: &str) -> (String, Option<f64>) {
    let trimmed = raw.trim();
    if let Some(at_pos) = trimmed.rfind('@') {
        let head = trimmed[..at_pos].trim();
        let tail = trimmed[at_pos + 1..].trim();
        if !head.is_empty() {
            if let Ok(parsed) = tail.parse::<f64>() {
                if parsed.is_finite() {
                    return (head.to_string(), Some(parsed));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}
