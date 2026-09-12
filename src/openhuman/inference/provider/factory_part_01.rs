use crate::openhuman::config::schema::cloud_providers::AuthStyle;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::auth::AuthStyle as CompatAuthStyle;
use crate::openhuman::inference::provider::claude_agent_sdk::subprocess::ClaudeAgentSdkProvider;
use crate::openhuman::inference::provider::openai_codex::{
    openai_codex_client_version, openai_codex_user_agent, resolve_openai_codex_routing,
    OPENAI_CODEX_ACCOUNT_HEADER, OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};
use crate::openhuman::inference::provider::openhuman_backend_model::OpenHumanBackendModel;
use crate::openhuman::inference::provider::ProviderRuntimeOptions;
use crate::openhuman::security::credentials::AuthService;
use std::sync::Arc;
use tinyinference::model::{ChatModel, ModelRequest, ModelResponse, ModelStream};

/// Sentinel meaning "use the OpenHuman backend session JWT".
pub const PROVIDER_OPENHUMAN: &str = "openhuman";
/// Prefix for Ollama-local providers: `"ollama:<model>"`.
pub const OLLAMA_PROVIDER_PREFIX: &str = "ollama:";
/// Prefix for LM Studio-local providers: `"lmstudio:<model>"`.
pub const LM_STUDIO_PROVIDER_PREFIX: &str = "lmstudio:";
/// Prefix for MLX-compatible local providers: `"mlx:<model>"`.
pub const MLX_PROVIDER_PREFIX: &str = "mlx:";
/// Prefix for OMLX local providers: `"omlx:<model>"`.
pub const OMLX_PROVIDER_PREFIX: &str = "omlx:";
/// Prefix for generic local OpenAI-compatible providers: `"local-openai:<model>"`.
pub const LOCAL_OPENAI_PROVIDER_PREFIX: &str = "local-openai:";
/// Prefix for the Claude Agent SDK subprocess provider: `"claude_agent_sdk:<model>"`.
pub const CLAUDE_AGENT_SDK_PREFIX: &str = "claude_agent_sdk:";
/// Sentinel for the Claude Agent SDK provider without a model suffix.
pub const CLAUDE_AGENT_SDK_PROVIDER: &str = "claude_agent_sdk";
/// Sentinel returned when a user has expressed custom/BYOK inference intent
/// (via a non-openhuman `inference_url`) but no matching `cloud_providers`
/// entry was found. Passed through `provider_for_role` and caught early in
/// `create_chat_model_from_string` to produce a clear configuration error
/// instead of silently routing through the managed OpenHuman backend.
pub const BYOK_INCOMPLETE_SENTINEL: &str = "__byok_incomplete__";

/// Interpolation-free substring of the empty-model bail emitted by
/// cloud-slug resolution when a `<slug>` provider string carries
/// no model and the `cloud_providers` entry has no `default_model` (the
/// #2784 guard). The Sentry-demotion + user-copy classifier
/// [`super::is_provider_config_rejection_message`] keys on this exact literal,
/// and a round-trip test in `factory_tests.rs` asserts the bail body still
/// contains it — so a wording drift fails CI instead of silently re-flooding
/// Sentry (TAURI-RUST-GKV).
pub(crate) const NO_MODEL_CONFIGURED_ANCHOR: &str = "resolved to an empty model id";

fn is_abstract_tier_model(model: &str) -> bool {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };
    let trimmed = model.trim();
    trimmed == MODEL_REASONING_V1
        || trimmed == MODEL_REASONING_QUICK_V1
        || trimmed == MODEL_CHAT_V1
        || trimmed == MODEL_AGENTIC_V1
        || trimmed == MODEL_BURST_V1
        || trimmed == MODEL_CODING_V1
        || trimmed == MODEL_VISION_V1
        || trimmed == MODEL_SUMMARIZATION_V1
}

/// Auth-profile storage key for a slug-keyed provider.
///
/// New writes use `"provider:<slug>"`. Lookups also try the bare `<slug>`
/// as a legacy fallback (old configs stored keys as e.g. `"openai:default"`).
pub fn auth_key_for_slug(slug: &str) -> String {
    format!("provider:{slug}")
}

/// Resolve a model hint (e.g. `"hint:reasoning"`) or tier name to the
/// concrete model string that the provider router would use — without
/// constructing the actual provider.  Returns the provider-string prefix
/// (e.g. `"openai"`) concatenated with the model when a BYOK provider is
/// active, or the bare tier name for the managed OpenHuman backend.
pub fn resolve_model_for_hint(hint_or_tier: &str, config: &Config) -> String {
    let hint_to_tier: &[(&str, &str)] = &[
        ("reasoning", crate::openhuman::config::MODEL_REASONING_V1),
        ("chat", crate::openhuman::config::MODEL_CHAT_V1),
        ("agentic", crate::openhuman::config::MODEL_AGENTIC_V1),
        ("burst", crate::openhuman::config::MODEL_BURST_V1),
        ("coding", crate::openhuman::config::MODEL_CODING_V1),
        ("vision", crate::openhuman::config::MODEL_VISION_V1),
        (
            "summarization",
            crate::openhuman::config::MODEL_SUMMARIZATION_V1,
        ),
        // Background subconscious workload rides the lightweight chat tier on the
        // managed backend; its `subconscious` *role* (handled below) still selects
        // the provider via `subconscious_provider`.
        ("subconscious", crate::openhuman::config::MODEL_CHAT_V1),
    ];
    let tier_to_role: &[(&str, &str)] = &[
        (crate::openhuman::config::MODEL_REASONING_V1, "reasoning"),
        (crate::openhuman::config::MODEL_CHAT_V1, "chat"),
        (crate::openhuman::config::MODEL_REASONING_QUICK_V1, "chat"),
        (crate::openhuman::config::MODEL_AGENTIC_V1, "agentic"),
        (crate::openhuman::config::MODEL_BURST_V1, "burst"),
        (crate::openhuman::config::MODEL_CODING_V1, "coding"),
        (crate::openhuman::config::MODEL_VISION_V1, "vision"),
        (
            crate::openhuman::config::MODEL_SUMMARIZATION_V1,
            "summarization",
        ),
    ];

    let (tier, role) = if let Some(hint_key) = hint_or_tier.strip_prefix("hint:") {
        let tier = hint_to_tier
            .iter()
            .find(|(k, _)| *k == hint_key)
            .map(|(_, v)| *v)
            .unwrap_or(hint_or_tier);
        // Background workloads map to a tier *model* but must keep their own
        // role so `provider_for_role` reads their dedicated `*_provider` field
        // rather than the chat-tier provider their model happens to share.
        let role = match hint_key {
            "subconscious" => "subconscious",
            _ => tier_to_role
                .iter()
                .find(|(k, _)| *k == tier)
                .map(|(_, v)| *v)
                .unwrap_or(hint_key),
        };
        (tier, role)
    } else {
        let role = tier_to_role
            .iter()
            .find(|(k, _)| *k == hint_or_tier)
            .map(|(_, v)| *v)
            .unwrap_or("chat");
        (hint_or_tier, role)
    };

    let provider_string = provider_for_role(role, config);
    let ps = provider_string.trim();
    if ps.is_empty() || ps == "cloud" || ps == PROVIDER_OPENHUMAN || ps == BYOK_INCOMPLETE_SENTINEL
    {
        tier.to_string()
    } else if let Some(idx) = ps.find(':') {
        let model_with_temp = &ps[idx + 1..];
        let (model, _) = split_model_and_temperature(model_with_temp);
        model
    } else {
        ps.to_string()
    }
}

/// Map a managed tier name (or `hint:*` string) to the workload **role** whose
/// configured provider serves it.
///
/// This is the inverse of the role→tier routing `create_chat_model` does:
/// callers that select a model *per unit of work by tier* (e.g. a tinyflows
/// `agent` node pinning `config.model = "reasoning-v1"`) use this to turn that
/// tier back into the role, then call [`create_chat_model`] with it — so the
/// completion routes to that tier on the managed backend (or the role's BYOK
/// model) instead of some caller default. Unknown strings fall back to `"chat"`.
///
/// Kept deliberately small and standalone (no `Config`) — it is a pure lookup
/// over the tier constants, mirroring the `tier_to_role` table inside
/// [`resolve_model_for_hint`].
pub fn role_for_model_tier(hint_or_tier: &str) -> &'static str {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };

    // Normalise a `hint:*` alias to its concrete tier first.
    let tier = match hint_or_tier.strip_prefix("hint:") {
        Some("reasoning") => MODEL_REASONING_V1,
        Some("chat") => MODEL_CHAT_V1,
        Some("agentic") => MODEL_AGENTIC_V1,
        Some("burst") => MODEL_BURST_V1,
        Some("coding") => MODEL_CODING_V1,
        Some("vision") => MODEL_VISION_V1,
        Some("summarization") => MODEL_SUMMARIZATION_V1,
        // Background subconscious rides the chat tier for its model.
        Some("subconscious") => MODEL_CHAT_V1,
        Some(_) => hint_or_tier,
        None => hint_or_tier,
    };

    match tier {
        MODEL_REASONING_V1 => "reasoning",
        MODEL_CHAT_V1 | MODEL_REASONING_QUICK_V1 => "chat",
        MODEL_AGENTIC_V1 => "agentic",
        MODEL_BURST_V1 => "burst",
        MODEL_CODING_V1 => "coding",
        MODEL_VISION_V1 => "vision",
        MODEL_SUMMARIZATION_V1 => "summarization",
        _ => "chat",
    }
}

/// Return whether `model` is a recognized OpenHuman backend tier name.
///
/// Used to guard against stale `default_model` values (e.g. set by older UI
/// versions) that the backend would reject with HTTP 400.  The known tiers are
/// the constants in `crate::openhuman::config`; the four `hint:*` strings that
/// `make_openhuman_backend` actually translates are also accepted.  An
/// unrecognized `hint:*` value is intentionally rejected so the factory falls
/// back to the platform default instead of forwarding an untranslated string
/// to the backend.
pub(crate) fn is_known_openhuman_tier(model: &str) -> bool {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };
    matches!(
        model,
        MODEL_REASONING_V1
            | MODEL_CHAT_V1
            | MODEL_AGENTIC_V1
            | MODEL_BURST_V1
            | MODEL_CODING_V1
            | MODEL_REASONING_QUICK_V1
            | MODEL_SUMMARIZATION_V1
            | MODEL_VISION_V1
            | "hint:reasoning"
            | "hint:chat"
            | "hint:agentic"
            | "hint:burst"
            | "hint:coding"
            | "hint:summarization"
            | "hint:vision"
    )
}

/// Return whether `model` is a raw BYOK/custom model id that must be forwarded
/// **verbatim** to provider construction rather than mapped onto a managed tier.
///
/// A raw passthrough id is any **non-empty** string that is neither a `hint:*`
/// alias nor a known managed tier ([`is_known_openhuman_tier`]) — i.e. the model
/// ids a user pins directly on an agent/node (e.g. `"claude-opus-4"`). The
/// OpenHuman backend preserves such ids verbatim
/// (the managed model's blank-id normalization) and is authoritative over
/// their validity, so the core must **not** silently collapse them onto
/// `reasoning-v1` (issue #4598). Managed tiers and every `hint:*` string return
/// `false` so their existing resolution is untouched.
pub(crate) fn is_raw_passthrough_model(model: &str) -> bool {
    let trimmed = model.trim();
    !trimmed.is_empty() && !trimmed.starts_with("hint:") && !is_known_openhuman_tier(trimmed)
}

/// Per-tier vision (image-input) capability for the managed OpenHuman backend.
///
/// The remote managed backend (`api.tinyhumans.ai`) does not advertise per-tier
/// capabilities, so the core maintains this map itself. Accepts both the tier
/// constants and their `hint:*` forms (callers may pass either pre- or
/// post-resolution).
///
/// `reasoning-v1` and the dedicated `vision-v1` tier are multimodal; every
/// other tier returns `false` — flip an individual arm to `true` once that tier
/// is confirmed multimodal on the backend. This is the **only** place to change
/// managed-model vision; BYOK/custom models are handled separately by the
/// user-set `model_registry.vision` flag
/// ([`crate::openhuman::inference::model_context::model_vision_enabled`]).
pub(crate) fn oh_tier_supports_vision(model: &str) -> bool {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
        MODEL_REASONING_V1, MODEL_SUMMARIZATION_V1, MODEL_VISION_V1,
    };
    match model {
        MODEL_REASONING_V1 | "hint:reasoning" => true,
        // Dedicated multimodal tier — the managed backend serves this with the
        // vision flag enabled. This is what the vision sub-agent rides on.
        MODEL_VISION_V1 | "hint:vision" => true,
        MODEL_CHAT_V1 | "hint:chat" => false,
        MODEL_REASONING_QUICK_V1 => false,
        MODEL_AGENTIC_V1 | "hint:agentic" => false,
        // Burst is a text-only tier.
        MODEL_BURST_V1 | "hint:burst" => false,
        MODEL_CODING_V1 | "hint:coding" => false,
        MODEL_SUMMARIZATION_V1 | "hint:summarization" => false,
        _ => false,
    }
}

/// The provider route a role has **explicitly** configured, before any
/// fallback.
///
/// Split out of [`provider_for_role`] so the fallback machinery can ask the
/// same question the router asks — "did the user route this role anywhere?" —
/// without re-deriving the role→config-field mapping and drifting from it.
fn configured_route_for_role<'a>(role: &str, config: &'a Config) -> Option<&'a str> {
    match role {
        "chat" => config.chat_provider.as_deref(),
        "reasoning" => config.reasoning_provider.as_deref(),
        "agentic" => config.agentic_provider.as_deref(),
        "coding" => config.coding_provider.as_deref(),
        // Burst uses the existing Agentic workload route for BYOK/local parity.
        // If unset, it falls through to the managed backend and is pinned to
        // `burst-v1` by `managed_tier_for_role`.
        "burst" => config.agentic_provider.as_deref(),
        // Tier-specific multimodal model; when unset it falls through to
        // `primary_cloud` (→ managed `vision-v1`), as every unset route now does.
        "vision" => config.vision_provider.as_deref(),
        // `memory_provider` covers both the memory-tree extract path and
        // the summarizer sub-agent (whose definition declares
        // `hint = "summarization"`). Both are "produce a condensed
        // representation of input text" — same model class, no reason
        // for a separate config knob.
        "memory" | "summarization" => config.memory_provider.as_deref(),
        "embeddings" => config.embeddings_provider.as_deref(),
        "heartbeat" => config.heartbeat_provider.as_deref(),
        "learning" => config.learning_provider.as_deref(),
        "subconscious" => config.subconscious_provider.as_deref(),
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
    if !super::fallback_diagnostics::role_falls_back_to_cloud(role) {
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
        if super::fallback_diagnostics::role_falls_back_to_cloud(role) {
            if let Some(chat) = config.chat_provider.as_deref() {
                if crate::openhuman::inference::local::profile::is_local_provider_string(chat) {
                    log::info!(
                        "[providers][local-fallback] role={} {}",
                        role,
                        super::fallback_diagnostics::cloud_fallback_notice(role, chat, &resolved)
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
fn route_has_usable_credentials(resolved: &str, config: &Config) -> bool {
    let r = resolved.trim();
    // Local runtimes (ollama/lmstudio/mlx/local-openai) and the local CLI
    // delegates carry their own credentials / run on-device.
    if crate::openhuman::inference::local::profile::is_local_provider_string(r)
        || r.starts_with(crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX)
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


/// Human-readable label for an *external* provider string, used in the
/// LocalOnly privacy-mode block message so the user knows what was refused.
fn external_provider_label(provider: &str) -> String {
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
    if p.starts_with(crate::openhuman::inference::provider::claude_code::PROVIDER_PREFIX) {
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
fn local_only_violation(
    mode: crate::openhuman::config::PrivacyMode,
    provider: &str,
) -> Option<String> {
    use crate::openhuman::config::PrivacyMode;
    if mode != PrivacyMode::LocalOnly {
        return None;
    }
    let p = provider.trim();
    if p.is_empty() || p == "cloud" {
        // Deferred: re-resolves to a concrete string on the recursive call.
        return None;
    }
    if crate::openhuman::inference::local::profile::is_local_provider_string(p) {
        return None;
    }
    Some(external_provider_label(p))
}

/// Enforce Privacy Mode `LocalOnly` at the inference chokepoint: refuse to build
/// an external chat provider when the live policy is local-only. Reads the live
/// privacy mode (defaults to `Standard`/allow when no session policy is
/// installed). See [`local_only_violation`] for the pure decision.
fn enforce_local_only_inference(role: &str, provider: &str) -> anyhow::Result<()> {
    let mode = crate::openhuman::security::live_policy::current_privacy_mode();
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
/// [`emit_external_transfer`](crate::openhuman::security::egress::emit_external_transfer)
/// discloses them without firing the external-transfer event.
fn emit_inference_egress(role: &str, provider: &str) {
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
    let is_local = crate::openhuman::inference::local::profile::is_local_provider_string(p);
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
    crate::openhuman::security::egress::emit_external_transfer(
        crate::openhuman::security::egress::EgressDescriptor::inference(slug, service, !is_local),
    );
}

/// Build an `Arc<dyn ChatModel>` for the given workload role.
///
/// The crate [`ChatModel`] is the model interface for the harness and one-shot
/// inference callers. Production and tests both inject this native interface;
/// `temperature` is applied as the request default while an explicit per-call
/// value still wins.
pub fn create_chat_model(
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    Ok(create_chat_model_with_model_id(role, config, temperature)?.0)
}

/// Like [`create_chat_model`], but also returns the resolved model id.
///
/// One-shot callers that persist or log the concrete model (e.g. the memory
/// summarise audit) need the id the role resolved to; the plain
/// [`create_chat_model`] drops it.
pub fn create_chat_model_with_model_id(
    role: &str,
    config: &Config,
    temperature: f64,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let (model, model_id) = create_chat_model_with_model_id_inner(role, config)?;
    Ok((with_default_temperature(model, temperature), model_id))
}
