//! Loading persisted config with a timeout, anchored reloads, and the
//! in-memory normalization that runs on every load.

use crate::core::runtime::DispatchCtx;
use std::path::Path;

use crate::config::Config;

/// What a SaaS call that carries no context is told, instead of the operator's config.
const NO_SAAS_CONFIG_SCOPE: &str =
    "[saas] no config in scope: SaaS work must run under a user agent's or the operator's context";

const CONFIG_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Loads persisted config with a 30s timeout.
///
/// This is used by JSON-RPC and CLI handlers to ensure they don't hang
/// indefinitely if disk I/O is blocked.
///
/// The TOML parse itself runs on the blocking pool via
/// `parse_config_with_recovery` (see `crates/openhuman-core/src/config/schema/load.rs`)
/// so the recursive-descent parser's serde Visitor frames don't compound
/// with whatever deep async tower called us. That's the stack-overflow
/// fix from `crahs.log` (2026-05-17); a per-call cache here would shave
/// the disk read on hot paths but proved racy across the in-process
/// integration tests (re-used workspace paths, concurrent server tasks
/// loading mid-mutation), so it isn't worth it.
/// An embedder-supplied config short-circuits the disk read entirely — see
/// [`CoreContext::embedder_config`](crate::core::runtime::context::CoreContext::embedder_config).
/// Without that branch, `CoreBuilder::config(..)` would configure boot and
/// nothing else: every handler calls this function per dispatch, so the turn
/// itself would still run against whatever the process-global workspace
/// resolution found. Normalization still runs, because that is a shaping step
/// handlers depend on, not a re-read.
pub async fn load_config_with_timeout() -> Result<Config, String> {
    // Transitional (#7801): callers that have no `DispatchCtx` of their own yet
    // read the task's ambient context. SaaS reads only the task's own scope, as
    // `Config::load_or_init` does, so a task that lost its user scope fails
    // rather than getting the operator's.
    load_config_in(&crate::core::runtime::DispatchCtx::ambient()).await
}

/// [`load_config_with_timeout`] for the context the caller was handed.
///
/// The context's embedder config, when it has one, short-circuits the disk
/// read (see [`CoreContext::embedder_config`](crate::core::runtime::context::CoreContext::embedder_config)).
/// In SaaS a call with no bound context fails instead of falling back to the
/// operator's config; outside SaaS an unbound context reads the process
/// default's, as an unscoped task always did.
pub async fn load_config_in(ctx: &crate::core::runtime::DispatchCtx) -> Result<Config, String> {
    if crate::core::runtime::is_saas() && ctx.core().is_none() {
        return Err(NO_SAAS_CONFIG_SCOPE.to_owned());
    }
    if let Some(mut config) = ctx.embedder_config() {
        normalize_loaded_config(&mut config).await;
        return Ok(config);
    }
    match tokio::time::timeout(CONFIG_LOAD_TIMEOUT, Config::load_or_init()).await {
        Ok(Ok(mut config)) => {
            normalize_loaded_config(&mut config).await;
            Ok(config)
        }
        // Surface the full anyhow chain (`{:#}`), not just the top `with_context`
        // line, so the underlying io error kind (e.g. `(os error 5)` access-denied
        // / `(os error 32)` sharing-lock) reaches Sentry. Without it the config
        // classifier and triage only ever see "Failed to read config file: <path>"
        // and cannot tell a user-environment denial from an app-side race
        // (#3962 / TAURI-RUST-DME).
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

/// The config of the ambient context: an embedded agent's own config under
/// its context, else the persisted config (initialised on first run).
///
/// The context-aware counterpart of [`Config::load_or_init`] for code that
/// runs inside a turn: [`load_config_with_timeout`] with the context's
/// inference route applied.
pub async fn load_current_or_init(ctx: &DispatchCtx) -> anyhow::Result<Config> {
    let mut config = ctx.load_config().await.map_err(anyhow::Error::msg)?;
    if let Some(route) = config.ephemeral_route.clone() {
        tracing::trace!(
            agent = crate::core::runtime::agent_scope::current_agent_id()
                .as_deref()
                .unwrap_or(""),
            "[config] applying the context's inference route"
        );
        crate::config::schema::ephemeral_route::apply(&mut config, route);
    }
    Ok(config)
}

/// Loads the config that belongs to `workspace_dir`, rather than whichever one
/// the process-global active-user / `OPENHUMAN_WORKSPACE` resolution currently
/// selects.
///
/// Use this from anything scoped to a workspace it was *handed* — the memory
/// subsystem driver is the first such caller. [`load_config_with_timeout`]
/// re-resolves the process-global workspace on every call, so a component bound
/// to workspace B that loads through it and then merely overwrites
/// `workspace_dir` keeps A's embedding routes, model dimensions and provider
/// credentials, and runs them against B's files.
///
/// The config file is looked for beside the workspace, in the two layouts the
/// resolver itself can produce: `<workspace>/config.toml` (a workspace root
/// that carries its own config) and `<workspace>/../config.toml` (the
/// `~/.openhuman/users/<id>/{config.toml,workspace}` layout). When neither
/// exists there is nothing workspace-specific to read, so this falls back to
/// the process-global load with `workspace_dir` re-anchored — the previous
/// behaviour, and still correct for a single-workspace host.
pub async fn load_config_for_workspace_with_timeout(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
) -> Result<Config, String> {
    let candidate = [
        workspace_dir.join("config.toml"),
        workspace_dir
            .parent()
            .map(|parent| parent.join("config.toml"))
            .unwrap_or_default(),
    ]
    .into_iter()
    .find(|path| path.is_file());

    if let Some(config_path) = candidate {
        tracing::debug!(
            config_path = %config_path.display(),
            workspace = %workspace_dir.display(),
            "[config] loading workspace-anchored config"
        );
        return match tokio::time::timeout(
            CONFIG_LOAD_TIMEOUT,
            Config::load_from_config_path(&config_path, workspace_dir),
        )
        .await
        {
            Ok(Ok(mut config)) => {
                normalize_loaded_config(&mut config).await;
                Ok(config)
            }
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(_) => Err("Config loading timed out".to_string()),
        };
    }

    tracing::debug!(
        workspace = %workspace_dir.display(),
        "[config] no config.toml beside workspace; falling back to the process-global load"
    );
    let mut config = ctx.load_config().await?;
    config.workspace_dir = workspace_dir.to_path_buf();
    Ok(config)
}

/// Reloads the config file represented by an existing runtime snapshot.
///
/// Use this for long-lived objects that need fresh config values while
/// staying anchored to their original user/workspace. Unlike
/// [`load_config_with_timeout`], this does not re-resolve the process-global
/// `OPENHUMAN_WORKSPACE` env var on every call.
pub async fn reload_config_snapshot_with_timeout(snapshot: &Config) -> Result<Config, String> {
    reload_config_from_paths(&snapshot.config_path, &snapshot.workspace_dir).await
}

/// The anchored reload, addressed by path rather than by a whole `Config`.
///
/// Callers that hold the extracted memory subsystem's `dyn MemoryHostConfig`
/// cannot produce a concrete `Config` to pass to
/// [`reload_config_snapshot_with_timeout`] — but they can read the two paths
/// off the seam. Same behaviour, narrower argument.
pub async fn reload_config_from_paths(
    config_path: &std::path::Path,
    workspace_dir: &std::path::Path,
) -> Result<Config, String> {
    match tokio::time::timeout(
        CONFIG_LOAD_TIMEOUT,
        Config::load_from_config_path(config_path, workspace_dir),
    )
    .await
    {
        Ok(Ok(mut config)) => {
            normalize_loaded_config(&mut config).await;
            Ok(config)
        }
        // Surface the full anyhow chain (`{:#}`), not just the top `with_context`
        // line, so the underlying io error kind (e.g. `(os error 5)` access-denied
        // / `(os error 32)` sharing-lock) reaches Sentry. Without it the config
        // classifier and triage only ever see "Failed to read config file: <path>"
        // and cannot tell a user-environment denial from an app-side race
        // (#3962 / TAURI-RUST-DME).
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

pub(crate) async fn normalize_loaded_config(config: &mut Config) {
    // Welcome-agent routing normalization removed (the welcome agent has been
    // deleted; all chat turns route directly to the orchestrator). The
    // `chat_onboarding_completed` field is retained only for backward-compatible
    // deserialization.

    if let Some(binding) = config.profile_providers.clone() {
        binding.apply_chat(config);
    }
    seed_and_enrich_model_registry(config);
    // Hook point for a host's resolved endpoints; covers the embedder-supplied
    // config path, which never goes through `Config::load_*`.
    crate::config::enrichers::run_enrichers(config);
}

/// Populate per-token pricing on the model registry from the static catalog.
///
/// Runs on every load and is **in-memory only** — it does not rewrite
/// `config.toml`. This keeps the user's persisted config clean (the catalog
/// stays the single source of truth, so price refreshes apply automatically)
/// while ensuring the Model Health dashboard, cost estimates, and the client
/// config snapshot see real numbers out of the box.
///
/// - Empty registry → seed it with one entry per catalogued model
///   ([`catalog::default_registry_entries`]).
/// - Otherwise → backfill any missing (zero) price on each existing entry,
///   preserving user-supplied prices and the `vision` flag
///   ([`catalog::enrich_entry`]).
///
/// Idempotent: re-running over an already-priced registry is a no-op.
pub(crate) fn seed_and_enrich_model_registry(config: &mut Config) {
    use crate::platform::cost::catalog;

    if config.model_registry.is_empty() {
        config.model_registry = catalog::default_registry_entries();
        log::debug!(
            "[config] seeded empty model_registry with {} catalogued models (as_of {})",
            config.model_registry.len(),
            tinyagents_registry::pricing::PRICING_AS_OF
        );
        return;
    }

    let mut filled = 0usize;
    for entry in &mut config.model_registry {
        if catalog::enrich_entry(entry) {
            filled += 1;
        }
    }
    if filled > 0 {
        log::debug!("[config] backfilled pricing on {filled} model_registry entries from catalog");
    }
}
