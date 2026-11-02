//! Agent, autonomy, paths, and memory-sync config operations.

use crate::core::runtime::DispatchCtx;

use crate::config::Config;
use crate::core::Outcome;

use super::loader::snapshot_config_json;
use super::tool_dispatcher as td;

/// Partial update for the `[autonomy]` block — the agent's filesystem access
/// mode. Each `None` field is left unchanged. `trusted_roots`, `allowed_commands`,
/// `forbidden_paths`, and `auto_approve`, when `Some`, REPLACE the corresponding
/// array wholesale.
#[derive(Debug, Clone, Default)]
pub struct AutonomySettingsPatch {
    /// Master switch for the whole autonomy policy. Defaults to `false`
    /// (`AutonomyConfig::enabled`): with it off, classification, the approval
    /// gate, the allowlist, the action budget and containment are all inert,
    /// and every other field in this patch has no effect until it is `true`.
    /// TinySecurity's protected-path floor applies either way.
    pub enabled: Option<bool>,
    /// `"readonly" | "supervised" | "full"` (case-insensitive).
    pub level: Option<String>,
    pub workspace_only: Option<bool>,
    pub allowed_commands: Option<Vec<String>>,
    pub forbidden_paths: Option<Vec<String>>,
    pub trusted_roots: Option<Vec<crate::security::TrustedRoot>>,
    pub allow_tool_install: Option<bool>,
    pub max_actions_per_hour: Option<u32>,
    /// "Always allow" allowlist — tool names the gate skips prompting for.
    pub auto_approve: Option<Vec<String>>,
    /// Blanket "auto-approve everything" bypass. `Unknown` origins are still
    /// denied by the gate regardless of this setting.
    pub auto_approve_all: Option<bool>,
}

/// Partial update for the `[agent]` block. Currently carries the single
/// user-facing `agent_timeout_secs` knob (the tool/action wall-clock timeout);
/// other `AgentConfig` fields are not yet UI-exposed. `None` leaves the value
/// unchanged.
#[derive(Debug, Clone, Default)]
pub struct AgentSettingsPatch {
    /// Tool/action wall-clock timeout in seconds. Validated to
    /// `tool_timeout::MIN_TIMEOUT_SECS..=tool_timeout::MAX_TIMEOUT_SECS`.
    pub agent_timeout_secs: Option<u64>,
    /// Agent the web-chat path routes a turn to (`[agent] chat_agent_id`).
    /// `Some("")`/whitespace clears the override and reverts to the
    /// orchestrator; `Some(id)` sets it; `None` leaves it unchanged.
    ///
    /// Settable over RPC and not only in the TOML because the file on disk is
    /// not reliably the file the core reads: once a user dir is active its
    /// per-user `config.toml` takes precedence, so a value pre-written to the
    /// root (or to a guessed user dir) is silently ignored. Going through the
    /// running core writes wherever `Config::save` actually points.
    pub chat_agent_id: Option<String>,
    /// How tool calls are spoken to the model (`[agent] tool_dispatcher`):
    /// one of [`TOOL_DISPATCHER_CHOICES`](super::tool_dispatcher::TOOL_DISPATCHER_CHOICES). `None` leaves it
    /// unchanged. Applies to new sessions; resumed threads keep their dialect.
    pub tool_dispatcher: Option<String>,
}

/// Updates the `[autonomy]` (agent access mode) settings in the configuration.
///
/// After saving, publishes a `DomainEvent::System(AutonomyConfigChanged)` so that
/// live agent sessions can rebuild their `SecurityPolicy` without a core restart
/// (see `channels::runtime`). Returns the updated config snapshot.
pub async fn apply_autonomy_settings(
    config: &mut Config,
    update: AutonomySettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    use crate::security::AutonomyLevel;

    // Validate before changing anything, so a rejected patch leaves `config` as it was.
    if let Some(root) = update
        .trusted_roots
        .iter()
        .flatten()
        .find(|root| !std::path::Path::new(&super::expand_tilde(&root.path)).is_absolute())
    {
        return Err(format!(
            "trusted root must be an absolute path (got {:?})",
            root.path
        ));
    }
    if let Some(enabled) = update.enabled {
        config.autonomy.enabled = enabled;
    }
    if let Some(level) = update.level {
        config.autonomy.level = match level.trim().to_ascii_lowercase().as_str() {
            "readonly" | "read_only" | "read-only" => AutonomyLevel::ReadOnly,
            "supervised" => AutonomyLevel::Supervised,
            "full" => AutonomyLevel::Full,
            other => {
                return Err(format!(
                    "invalid autonomy level '{other}' (expected readonly | supervised | full)"
                ));
            }
        };
    }
    if let Some(workspace_only) = update.workspace_only {
        config.autonomy.workspace_only = workspace_only;
    }
    if let Some(allowed_commands) = update.allowed_commands {
        config.autonomy.allowed_commands = allowed_commands;
    }
    if let Some(forbidden_paths) = update.forbidden_paths {
        config.autonomy.forbidden_paths = forbidden_paths;
    }
    if let Some(trusted_roots) = update.trusted_roots {
        config.autonomy.trusted_roots = trusted_roots;
    }
    if let Some(allow_tool_install) = update.allow_tool_install {
        config.autonomy.allow_tool_install = allow_tool_install;
    }
    if let Some(max_actions_per_hour) = update.max_actions_per_hour {
        if max_actions_per_hour == 0 {
            return Err(format!(
                "max_actions_per_hour must be at least 1 (got {max_actions_per_hour})"
            ));
        }
        config.autonomy.max_actions_per_hour = max_actions_per_hour;
    }
    if let Some(auto_approve) = update.auto_approve {
        config.autonomy.auto_approve = auto_approve;
    }
    if let Some(auto_approve_all) = update.auto_approve_all {
        config.autonomy.auto_approve_all = auto_approve_all;
    }

    config.save().await.map_err(super::config_save_error)?;

    crate::security::live_policy::reload_from(&config.autonomy);
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::AutonomyConfigChanged);

    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "autonomy settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies autonomy settings updates, and saves it.
pub async fn load_and_apply_autonomy_settings(
    ctx: &DispatchCtx,
    update: AutonomySettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_autonomy_settings(&mut config, update).await
}

/// Returns the current `[autonomy]` settings block as JSON (no secrets).
pub async fn get_autonomy_settings(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let value = serde_json::to_value(&config.autonomy).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(value, "autonomy settings read"))
}

fn auto_approve_write_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Append `tool_name` to `autonomy.auto_approve` ("Always allow") and persist +
/// reload the live policy. Idempotent — a no-op (no disk write) when the tool is
/// already allow-listed. Backs the `ApproveAlwaysForTool` approval decision.
pub async fn add_auto_approve_tool(ctx: &DispatchCtx, tool_name: &str) -> Result<(), String> {
    let _guard = auto_approve_write_lock().lock().await;
    let mut config = ctx.load_config().await?;
    if config.autonomy.auto_approve.iter().any(|t| t == tool_name) {
        tracing::debug!(
            tool = tool_name,
            "[config:auto_approve] tool already allow-listed; nothing to persist"
        );
        return Ok(());
    }
    let mut next = config.autonomy.auto_approve.clone();
    next.push(tool_name.to_string());
    let patch = AutonomySettingsPatch {
        auto_approve: Some(next),
        ..AutonomySettingsPatch::default()
    };
    apply_autonomy_settings(&mut config, patch)
        .await
        .map(|_| ())
}

/// Updates the `[agent]` block (currently the `agent_timeout_secs` tool/action
/// wall-clock timeout).
///
/// After persisting, pushes the new value into the live
/// [`crate::tools::timeout`] runtime so subsequent tool calls honour
/// it without a core restart. The `OPENHUMAN_TOOL_TIMEOUT_SECS` env var, when
/// set, still overrides the config value (the push is a no-op in that case).
/// Returns the updated config snapshot.
pub async fn apply_agent_settings(
    config: &mut Config,
    update: AgentSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    use tinyagents_harness_tools::exec::timeout::{MAX_TIMEOUT_SECS, MIN_TIMEOUT_SECS};

    if let Some(timeout_secs) = update.agent_timeout_secs {
        if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&timeout_secs) {
            log::warn!(
                "[config][agent] rejected agent_timeout_secs={timeout_secs} (valid {MIN_TIMEOUT_SECS}..={MAX_TIMEOUT_SECS})"
            );
            return Err(format!(
                "agent_timeout_secs must be between {MIN_TIMEOUT_SECS} and {MAX_TIMEOUT_SECS} seconds (got {timeout_secs})"
            ));
        }
    }

    if let Some(chat_agent_id) = update.chat_agent_id.as_deref() {
        let trimmed = chat_agent_id.trim();
        if !trimmed.is_empty()
            && !crate::agent::OpenHumanSessionHost::is_runnable_agent_id(config, trimmed)
        {
            return Err(format!(
                "chat_agent_id '{trimmed}' is not a runnable agent definition"
            ));
        }
    }

    let tool_dispatcher = td::normalize_optional(update.tool_dispatcher.as_deref())?;

    if let Some(timeout_secs) = update.agent_timeout_secs {
        config.agent.agent_timeout_secs = timeout_secs;
    }

    td::apply_tool_dispatcher(config, tool_dispatcher);

    if let Some(chat_agent_id) = update.chat_agent_id {
        let trimmed = chat_agent_id.trim();
        config.agent.chat_agent_id = (!trimmed.is_empty()).then(|| trimmed.to_string());
        log::debug!(
            "[config][agent] chat_agent_id -> {:?}",
            config.agent.chat_agent_id
        );
    }

    config.save().await.map_err(super::config_save_error)?;

    let effective = crate::tools::timeout::set_tool_timeout_secs(config.agent.agent_timeout_secs);
    log::debug!(
        "[config][agent] agent settings saved; agent_timeout_secs={} effective={}s",
        config.agent.agent_timeout_secs,
        effective
    );

    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "agent settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies agent settings updates, and saves it.
pub async fn load_and_apply_agent_settings(
    ctx: &DispatchCtx,
    update: AgentSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_agent_settings(&mut config, update).await
}

/// Returns the agent execution settings (currently the action timeout) plus the
/// runtime-effective value and whether the `OPENHUMAN_TOOL_TIMEOUT_SECS` env var
/// is overriding the configured value, so the UI can explain a no-op control.
pub async fn get_agent_settings(ctx: &DispatchCtx) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    crate::tools::timeout::set_tool_timeout_secs(config.agent.agent_timeout_secs);
    let value = serde_json::json!({
        "agent_timeout_secs": config.agent.agent_timeout_secs,
        "effective_timeout_secs": crate::tools::timeout::tool_execution_timeout_secs(),
        "env_override": crate::tools::timeout::env_override_active(),
        "min_timeout_secs": tinyagents_harness_tools::exec::timeout::MIN_TIMEOUT_SECS,
        "max_timeout_secs": tinyagents_harness_tools::exec::timeout::MAX_TIMEOUT_SECS,
        "tool_dispatcher": config.agent.tool_dispatcher,
        "tool_dispatcher_env_override": td::tool_dispatcher_env_override(),
    });
    Ok(Outcome::single_log(value, "agent settings read"))
}
