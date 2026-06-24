//! Reads `GMI_MAAS_*` env vars injected by AgentBox at runtime and registers
//! an OpenAI-compatible cloud provider so the agent runtime can call the
//! marketplace's MaaS inference endpoint.

use std::collections::HashMap;

use crate::openhuman::config::schema::cloud_providers::{
    generate_provider_id, AuthStyle, CloudProviderCreds,
};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::inference::provider::factory::auth_key_for_slug;

/// Slug used to identify the GMI MaaS provider in `Config::cloud_providers`
/// and in auth-profiles (keyed by `provider:<slug>`).
pub const GMI_MAAS_SLUG: &str = "gmi-maas";

/// Hard upper bound on how long startup will wait for the GMI provider patch
/// (config load + token store + config save). The happy path is a few
/// filesystem ops (<100ms), but `AuthProfilesStore` lock acquisition can busy-
/// wait up to ~35s under contention. We must not let that stall server
/// readiness / liveness probes, so we cap the budget and fall through to
/// degraded mode on timeout — `/run` then surfaces a clear provider error
/// rather than the boot hanging.
const GMI_REGISTRATION_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// Operator-supplied flag that turns the marketplace adapter on. When set to
/// `"1"` the core mounts `/run` + `/jobs/{id}` and routes inference through the
/// GMI MaaS provider seeded from `GMI_*` env vars.
pub const AGENTBOX_MODE_ENV_VAR: &str = "OPENHUMAN_AGENTBOX_MODE";

#[cfg(test)]
pub(crate) fn agentbox_env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether the core is running as a GMI Cloud AgentBox marketplace container.
///
/// Single source of truth for the `OPENHUMAN_AGENTBOX_MODE=1` check so the
/// router mount, startup registration, and the inference session-gate bypass
/// can't drift apart.
pub fn agentbox_mode_enabled() -> bool {
    std::env::var(AGENTBOX_MODE_ENV_VAR).as_deref() == Ok("1")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// Collect GMI config from a getter (real env or test fake).
///
/// Returns `Ok(_)` only when all three vars are present and non-blank. The
/// error string lists every missing var so the operator can fix all at once.
pub fn collect_gmi_config<F>(get: F) -> Result<GmiConfig, String>
where
    F: Fn(&str) -> Option<String>,
{
    let base_url = nonblank(&get, "GMI_MAAS_BASE_URL");
    let api_key = nonblank(&get, "GMI_MAAS_API_KEY");
    let model = nonblank(&get, "GMI_MODELS");

    let mut missing = Vec::new();
    if base_url.is_none() {
        missing.push("GMI_MAAS_BASE_URL");
    }
    if api_key.is_none() {
        missing.push("GMI_MAAS_API_KEY");
    }
    if model.is_none() {
        missing.push("GMI_MODELS");
    }
    if !missing.is_empty() {
        return Err(format!("missing/blank: {}", missing.join(", ")));
    }
    Ok(GmiConfig {
        base_url: base_url.unwrap(),
        api_key: api_key.unwrap(),
        model: model.unwrap(),
    })
}

fn nonblank<F: Fn(&str) -> Option<String>>(get: &F, key: &str) -> Option<String> {
    get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Read env and register the GMI MaaS provider on startup if available.
///
/// No-op (with a warning log) if any required var is missing — the core
/// still boots in degraded mode, useful for local testing of `/run` without
/// GMI.
///
/// **Never logs the API key.**
pub fn register_gmi_provider_if_present() {
    let cfg = match collect_gmi_config(|k| std::env::var(k).ok()) {
        Ok(cfg) => cfg,
        Err(reason) => {
            // Only surface as a warning when AgentBox mode is actually enabled.
            // Otherwise (desktop / CLI default), this is the expected steady
            // state and operators would treat the warn as noise.
            if agentbox_mode_enabled() {
                log::warn!(
                    "[agentbox::gmi] not registering GMI MaaS provider: {}",
                    reason
                );
            } else {
                log::debug!(
                    "[agentbox::gmi] not registering GMI MaaS provider (AgentBox mode off): {}",
                    reason
                );
            }
            return;
        }
    };

    log::info!(
        "[agentbox::gmi] registering provider base_url={} model={}",
        cfg.base_url,
        cfg.model
    );

    register_gmi_with_inference_catalog(&cfg);
}

/// Install a `gmi-maas` entry into `config.cloud_providers`, store the API key
/// in the auth-profile store keyed by `provider:gmi-maas`, and point every
/// agent-runtime workload at `gmi-maas:<model>` so all inference is routed to
/// the marketplace MaaS endpoint.
///
/// Runs the async config load/save on the current Tokio runtime via
/// `block_in_place` (we are called from inside `run_server_inner` on a
/// multi-threaded runtime, so this is safe and lets us keep the public
/// `register_gmi_provider_if_present()` API synchronous).
///
/// All failures are logged and swallowed — startup must never panic on a
/// missing/broken config; the operator can still recover by editing
/// `config.toml` manually.
fn register_gmi_with_inference_catalog(cfg: &GmiConfig) {
    // We need an async context for `Config::load_or_init` + `config.save`.
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(e) => {
            log::warn!(
                "[agentbox::gmi] no current tokio runtime — skipping GMI MaaS \
                 provider registration ({}). The bridge runs from \
                 `run_server_inner` which is always async; this branch only \
                 fires in unusual embedding scenarios.",
                e
            );
            return;
        }
    };

    // `block_in_place` panics on a current-thread runtime — only multi-thread
    // runtimes support it. Today's callers (`run_server_inner` from `cli.rs`
    // and `lib.rs`) are multi-thread, but the contract says this function must
    // never panic. Detect the flavor up front and bail with a warning if a
    // current-thread runtime is in use (consistent with the no-runtime branch).
    if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
        log::warn!(
            "[agentbox::gmi] current-thread tokio runtime detected — skipping \
             GMI MaaS provider registration (multi-thread runtime required for \
             block_in_place)"
        );
        return;
    }

    let cfg_clone = cfg.clone();
    // Bound the synchronous wait: we keep the ordering contract (the provider
    // must be in the catalog before `/run` is mounted) but cap how long boot
    // can block so auth-store lock contention can't stall readiness for ~35s.
    let result = tokio::task::block_in_place(|| {
        handle.block_on(async move {
            tokio::time::timeout(GMI_REGISTRATION_BUDGET, apply_gmi_to_runtime(&cfg_clone)).await
        })
    });

    match result {
        Ok(Ok(())) => {
            log::info!(
                "[agentbox::gmi] registered cloud provider slug={} model={} \
                 base_url={} — all agent workloads routed to gmi-maas",
                GMI_MAAS_SLUG,
                cfg.model,
                cfg.base_url,
            );
        }
        Ok(Err(e)) => {
            log::warn!(
                "[agentbox::gmi] GMI MaaS registration failed (startup continues \
                 in degraded mode): {}",
                e
            );
        }
        Err(_elapsed) => {
            log::warn!(
                "[agentbox::gmi] GMI MaaS registration exceeded {}s startup budget \
                 (likely auth-store lock contention) — startup continues in \
                 degraded mode; /run will report a provider error until retried",
                GMI_REGISTRATION_BUDGET.as_secs(),
            );
        }
    }
}

async fn apply_gmi_to_runtime(cfg: &GmiConfig) -> Result<(), String> {
    log::debug!("[agentbox::gmi] loading config for in-place provider patch");
    let mut config = Config::load_or_init().await.map_err(|e| e.to_string())?;

    // 1. Upsert the `gmi-maas` cloud_providers entry. Preserve the stable id
    //    if an entry already exists (idempotent across restarts).
    let existing_id = config
        .cloud_providers
        .iter()
        .find(|e| e.slug == GMI_MAAS_SLUG)
        .map(|e| e.id.clone());
    let id = existing_id.unwrap_or_else(|| generate_provider_id(GMI_MAAS_SLUG));
    let entry = CloudProviderCreds {
        id: id.clone(),
        slug: GMI_MAAS_SLUG.to_string(),
        label: "GMI MaaS (AgentBox)".to_string(),
        endpoint: cfg.base_url.clone(),
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    };
    if let Some(existing) = config
        .cloud_providers
        .iter_mut()
        .find(|e| e.slug == GMI_MAAS_SLUG)
    {
        *existing = entry;
        log::debug!("[agentbox::gmi] updated existing cloud_providers entry id={id}");
    } else {
        config.cloud_providers.push(entry);
        log::debug!("[agentbox::gmi] inserted new cloud_providers entry id={id}");
    }

    // 2. Wire every agent-runtime provider slot to `gmi-maas:<model>` so the
    //    factory routes inference through this entry. AgentBox-managed runs
    //    must never silently fall through to a locally-configured provider.
    let provider_string = format!("{}:{}", GMI_MAAS_SLUG, cfg.model);
    config.primary_cloud = Some(id.clone());
    config.chat_provider = Some(provider_string.clone());
    config.reasoning_provider = Some(provider_string.clone());
    config.agentic_provider = Some(provider_string.clone());
    config.coding_provider = Some(provider_string.clone());
    config.memory_provider = Some(provider_string.clone());
    config.heartbeat_provider = Some(provider_string.clone());
    config.learning_provider = Some(provider_string.clone());
    config.subconscious_provider = Some(provider_string);

    // 3. Store the API key BEFORE persisting the config to disk. If the token
    //    write fails we don't want config.toml to advertise a `gmi-maas` entry
    //    whose credential lookup will 401 the next /run. The reverse order
    //    (current pre-fix code) is harder to recover from than an orphaned
    //    token, which is overwritten cleanly on the next idempotent re-register.
    //    NOTE: never log `cfg.api_key`.
    let auth = AuthService::from_config(&config);
    let auth_key = auth_key_for_slug(GMI_MAAS_SLUG);
    auth.store_provider_token(
        &auth_key,
        "default",
        &cfg.api_key,
        HashMap::new(),
        /* set_active */ true,
    )
    .map_err(|e| format!("store_provider_token failed: {e}"))?;
    log::debug!(
        "[agentbox::gmi] stored API key in auth profile store keyed by {}",
        auth_key
    );

    config.save().await.map_err(|e| e.to_string())?;
    log::debug!(
        "[agentbox::gmi] config saved to {}",
        config.config_path.display()
    );

    Ok(())
}
