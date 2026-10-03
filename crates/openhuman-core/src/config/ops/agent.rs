//! Agent, autonomy, paths, and memory-sync config operations.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::config::Config;
use crate::core::Outcome;

use super::loader::{load_config_with_timeout, snapshot_config_json};
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
    /// `is_always_forbidden` applies either way.
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

/// Partial update for the agent's editable filesystem roots.
///
/// `action_dir` (issue #3240) and the files folder (#5505) are editable.
/// `workspace_dir` and `projects_dir` are intentionally read-only.
#[derive(Debug, Clone, Default)]
pub struct AgentPathsPatch {
    /// New action sandbox root. `Some("")`/whitespace clears the override and
    /// reverts to the default; `Some(path)` sets it; `None` leaves it unchanged.
    pub action_dir: Option<String>,
    /// New folder for agent deliverables. Same `Some("")` / `Some(path)` /
    /// `None` semantics as `action_dir`. Affects new artifacts only.
    pub files_dir: Option<String>,
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
                ))
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

    config.save().await.map_err(|e| e.to_string())?;

    crate::security::live_policy::reload_from(&config.autonomy);
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::AutonomyConfigChanged);

    let snapshot = snapshot_config_json(config)?;
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
    update: AutonomySettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_autonomy_settings(&mut config, update).await
}

/// Returns the current `[autonomy]` settings block as JSON (no secrets).
pub async fn get_autonomy_settings() -> Result<Outcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
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
pub async fn add_auto_approve_tool(tool_name: &str) -> Result<(), String> {
    let _guard = auto_approve_write_lock().lock().await;
    let mut config = load_config_with_timeout().await?;
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
    use crate::tools::timeout::{MAX_TIMEOUT_SECS, MIN_TIMEOUT_SECS};

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

    config.save().await.map_err(|e| e.to_string())?;

    let effective = crate::tools::timeout::set_tool_timeout_secs(config.agent.agent_timeout_secs);
    log::debug!(
        "[config][agent] agent settings saved; agent_timeout_secs={} effective={}s",
        config.agent.agent_timeout_secs,
        effective
    );

    let snapshot = snapshot_config_json(config)?;
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
    update: AgentSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_agent_settings(&mut config, update).await
}

/// Returns the agent execution settings (currently the action timeout) plus the
/// runtime-effective value and whether the `OPENHUMAN_TOOL_TIMEOUT_SECS` env var
/// is overriding the configured value, so the UI can explain a no-op control.
pub async fn get_agent_settings() -> Result<Outcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    crate::tools::timeout::set_tool_timeout_secs(config.agent.agent_timeout_secs);
    let value = serde_json::json!({
        "agent_timeout_secs": config.agent.agent_timeout_secs,
        "effective_timeout_secs": crate::tools::timeout::tool_execution_timeout_secs(),
        "env_override": crate::tools::timeout::env_override_active(),
        "min_timeout_secs": crate::tools::timeout::MIN_TIMEOUT_SECS,
        "max_timeout_secs": crate::tools::timeout::MAX_TIMEOUT_SECS,
        "tool_dispatcher": config.agent.tool_dispatcher,
        "tool_dispatcher_env_override": td::tool_dispatcher_env_override(),
    });
    Ok(Outcome::single_log(value, "agent settings read"))
}

/// Expand a leading `~/` to the user's home directory, building the path
/// component-by-component so the result uses the platform-native separator
/// throughout. A naive `format!("{}/{rest}", home)` — or even `home.join(rest)`
/// — leaves the embedded `/` inside `rest`, yielding a mixed-separator path like
/// `C:\Users\Harry/OpenHuman/projects` on Windows, which `CreateProcessW`
/// rejects with `ERROR_DIRECTORY` (os error 267) when used as a process CWD.
/// See issue #3353 (RC-B).
///
/// This is the single source of truth for `~/` expansion; `SecurityPolicy::
/// expand_tilde` delegates here so policy and config stay byte-for-byte
/// consistent.
pub fn expand_tilde(path: &str) -> String {
    let Some(rest) = path.strip_prefix("~/") else {
        return path.to_string();
    };
    let Some(home) = dirs::home_dir() else {
        return path.to_string();
    };
    let mut out = home;
    for part in rest.split('/') {
        if !part.is_empty() {
            out.push(part);
        }
    }
    out.to_string_lossy().into_owned()
}

/// Redact a path for logging by replacing the user's home-directory prefix with
/// `~`. Keeps the path *shape* (e.g. `~/OpenHuman/projects`) useful for
/// diagnosis while not leaking the OS username / full home path (PII). Paths
/// outside the home dir are returned unchanged.
pub fn redact_home(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if !home.is_empty() {
            if let Some(rest) = s.strip_prefix(home.as_ref()) {
                return format!("~{rest}");
            }
        }
    }
    s.into_owned()
}

/// Ensure the agent's action sandbox + default projects home exist and the
/// projects dir is registered as a `ReadWrite` trusted root. Idempotent — safe
/// to call from every boot path (web-chat-only `bootstrap_core_runtime` **and**
/// `start_channels`).
///
/// Without this on the always-run boot, a fresh desktop install with no
/// messaging integrations leaves `~/OpenHuman/projects` uncreated (the only
/// other creation lived inside the integration-gated `start_channels`), so the
/// shell tool's `current_dir` fails with `ERROR_DIRECTORY` (os error 267) on
/// Windows / `ENOENT` on Unix. See issue #3353 (RC-A).
pub async fn ensure_agent_dirs(config: &mut Config) {
    use crate::security::{TrustedAccess, TrustedRoot};

    let projects_dir = crate::config::default_projects_dir();
    if let Err(e) = tokio::fs::create_dir_all(&projects_dir).await {
        tracing::warn!(
            dir = %redact_home(&projects_dir),
            error = %e,
            "[startup] could not create default projects dir"
        );
    }
    let projects_path = projects_dir.to_string_lossy().to_string();
    if !config
        .autonomy
        .trusted_roots
        .iter()
        .any(|r| r.path == projects_path)
    {
        config.autonomy.trusted_roots.push(TrustedRoot {
            path: projects_path,
            access: TrustedAccess::ReadWrite,
        });
    }

    let action_dir = config.action_dir.clone();
    if let Err(e) = tokio::fs::create_dir_all(&action_dir).await {
        tracing::warn!(
            dir = %redact_home(&action_dir),
            error = %e,
            "[startup] could not create action sandbox dir"
        );
    }
    // Creating the action dir is not the same as being allowed to write in it:
    // `validate_path` only *joins* relative tool paths onto it, and the
    // permission comes from a trusted root. `SecurityPolicy::from_config` grants
    // it, and this is the config-side mirror so a persisted config carries the
    // same grant. Skipped when the action dir sits at or above `workspace_dir`,
    // where a trusted root would buy a `forbidden_paths` bypass over the whole
    // workspace.
    let action_path = action_dir.to_string_lossy().to_string();
    if !action_path.is_empty()
        && !config.workspace_dir.starts_with(&action_dir)
        && !config
            .autonomy
            .trusted_roots
            .iter()
            .any(|r| r.path == action_path)
    {
        config.autonomy.trusted_roots.push(TrustedRoot {
            path: action_path,
            access: TrustedAccess::ReadWrite,
        });
    }

    tracing::info!(
        workspace = %redact_home(&config.workspace_dir),
        action = %redact_home(&action_dir),
        "[startup] workspace (internal state) and action sandbox (tool cwd) directories configured"
    );

    // Agent deliverables are written to the visible files folder (#5505).
    // Create it up front, and move this account's pre-#5505 artifact files out
    // of the hidden workspace (idempotent; a no-op once migrated).
    let files_dir = config.files_dir();
    if let Err(e) = tokio::fs::create_dir_all(&files_dir).await {
        tracing::warn!(
            dir = %redact_home(&files_dir),
            error = %e,
            "[startup] could not create files folder"
        );
    }
    let report =
        crate::agent::artifacts::migrate_legacy_artifacts(&config.workspace_dir, &files_dir).await;
    if report != crate::agent::artifacts::MigrationReport::default() {
        tracing::info!(
            moved = report.moved,
            cleaned = report.cleaned,
            failed = report.failed,
            "[startup] moved legacy artifact files into the files folder"
        );
    }
}

/// Ensure `dir` is usable as a process working directory: it must exist (we
/// attempt to create it if missing — covers a dir deleted after launch) and
/// resolve to a directory. Returns a descriptive error naming the path and the
/// Settings location to fix it, instead of letting the OS surface an opaque
/// `ERROR_DIRECTORY` (os error 267) from `CreateProcessW`. See issue #3353
/// (Fix 2). Cheap stat-only calls on the happy path.
pub fn ensure_usable_cwd(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| {
            anyhow::anyhow!(
                "Working directory '{}' does not exist and could not be created: {e}. \
                 Set a valid path in Settings → Agent access → Working directory.",
                dir.display()
            )
        })?;
    }
    if !dir.is_dir() {
        anyhow::bail!(
            "Working directory '{}' is not a directory. \
             Set a valid path in Settings → Agent access → Working directory.",
            dir.display()
        );
    }
    Ok(())
}

fn action_dir_source(config: &Config) -> &'static str {
    if crate::config::action_dir_env_override().is_some() {
        "env"
    } else if config.action_dir_override.is_some() {
        "override"
    } else {
        "default"
    }
}

fn agent_paths_payload(config: &Config) -> serde_json::Value {
    let projects_dir = crate::config::default_projects_dir();
    json!({
        "action_dir": config.action_dir.display().to_string(),
        "workspace_dir": config.workspace_dir.display().to_string(),
        "projects_dir": projects_dir.display().to_string(),
        "action_dir_source": action_dir_source(config),
        "files_dir": config.files_dir().display().to_string(),
        "default_files_dir": crate::config::default_files_dir().display().to_string(),
        "files_dir_source": if config.files_dir_override.is_some() { "override" } else { "default" },
    })
}

/// Validate a user-chosen files folder (#5505) and create it. Fail-closed:
/// the path must be absolute, not an existing file, not a protected location
/// (credential stores, OS directories), and not inside the OpenHuman data
/// directory — the point of the folder is that the user can see it, and the
/// data dir holds internal state.
async fn validate_files_dir(raw: &str, config: &Config) -> Result<PathBuf, String> {
    let expanded = expand_tilde(raw);
    let candidate = PathBuf::from(&expanded);
    if !candidate.is_absolute() {
        return Err(format!(
            "files_dir must be an absolute path (got '{expanded}')"
        ));
    }
    if candidate.is_file() {
        return Err(format!(
            "files_dir must be a folder, not a file: {expanded}"
        ));
    }
    if crate::security::SecurityPolicy::is_always_forbidden(&candidate) {
        return Err(format!(
            "files_dir cannot be a protected system or credential folder: {expanded}"
        ));
    }
    let mut internal = vec![config.workspace_dir.clone()];
    if let Ok(root) = crate::config::default_root_openhuman_dir() {
        internal.push(root);
    }
    if internal.iter().any(|dir| path_within(&candidate, dir)) {
        return Err(format!(
            "files_dir must not be inside the OpenHuman data folder: {expanded}"
        ));
    }
    tokio::fs::create_dir_all(&candidate)
        .await
        .map_err(|e| format!("failed to create files_dir {expanded}: {e}"))?;
    Ok(candidate)
}

/// `path` equals or sits under `dir`, comparing canonical forms so a
/// symlinked spelling (e.g. macOS `/var` → `/private/var`) cannot slip into
/// the data dir. A path that does not exist yet is canonicalized through its
/// nearest existing ancestor.
fn path_within(path: &Path, dir: &Path) -> bool {
    canonical_prefix(path).starts_with(canonical_prefix(dir))
}

fn canonical_prefix(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(canon) = cursor.canonicalize() {
            return rest.iter().rev().fold(canon, |acc, part| acc.join(part));
        }
        match (cursor.parent(), cursor.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cursor = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Applies an edit to the agent's `action_dir` sandbox root.
///
/// Validation (fail-closed): the path is trimmed and `~`-expanded; it must be
/// **absolute**; it must not be an existing *file*; and it must not equal
/// `workspace_dir` (which holds memory DBs / tokens and must never become the
/// agent-writable root). A missing directory is auto-created (mirroring the
/// startup auto-create in `channels/runtime/startup.rs`). An empty input clears
/// the override and reverts `action_dir` to the default.
///
/// On success the override is persisted (`action_dir_override`), `action_dir` is
/// recomputed from the precedence chain, the live `SecurityPolicy` is hot-swapped
/// (`live_policy::set_action_dir`), and `DomainEvent::AgentPathsChanged` is
/// published. Returns the same payload shape as [`get_agent_paths`].
///
/// When `OPENHUMAN_ACTION_DIR` is set the env var wins: the override is still
/// persisted, but the effective `action_dir` (and the returned `action_dir`)
/// continues to reflect the env value, and `action_dir_source` reports `"env"`.
pub async fn apply_agent_paths_settings(
    config: &mut Config,
    update: AgentPathsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut notes: Vec<String> = Vec::new();

    if let Some(raw) = update.action_dir {
        let trimmed = raw.trim();
        log::debug!(
            "[config][agent_paths] apply action_dir edit (input_len={})",
            trimmed.len()
        );

        if trimmed.is_empty() {
            config.action_dir_override = None;
            notes.push("action_dir override cleared (reverted to default)".to_string());
        } else {
            let expanded = expand_tilde(trimmed);
            let candidate = PathBuf::from(&expanded);

            if !candidate.is_absolute() {
                return Err(format!(
                    "action_dir must be an absolute path (got '{expanded}')"
                ));
            }

            if candidate.is_file() {
                return Err(format!(
                    "action_dir must be a directory, not a file: {expanded}"
                ));
            }

            if paths_equal(&candidate, &config.workspace_dir) {
                return Err(
                    "action_dir must not equal the internal workspace directory".to_string()
                );
            }

            if !candidate.exists() {
                tokio::fs::create_dir_all(&candidate)
                    .await
                    .map_err(|e| format!("failed to create action_dir {expanded}: {e}"))?;
                notes.push(format!("created action_dir {expanded}"));
            }

            config.action_dir_override = Some(candidate);
            notes.push(format!("action_dir override set to {expanded}"));
        }

        config.action_dir = crate::config::resolve_action_dir(&config.action_dir_override);

        config.save().await.map_err(|e| e.to_string())?;

        crate::security::live_policy::set_action_dir(config.action_dir.clone());
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::AgentPathsChanged);

        log::debug!(
            "[config][agent_paths] action_dir now '{}' (source={})",
            config.action_dir.display(),
            action_dir_source(config)
        );
    }

    if let Some(raw) = update.files_dir {
        let trimmed = raw.trim();
        log::debug!(
            "[config][agent_paths] apply files_dir edit (input_len={})",
            trimmed.len()
        );
        let previous = config.files_dir();
        if trimmed.is_empty() {
            config.files_dir_override = None;
            notes.push("files_dir override cleared (reverted to default)".to_string());
        } else {
            let candidate = validate_files_dir(trimmed, config).await?;
            notes.push(format!("files_dir set to {}", candidate.display()));
            config.files_dir_override = Some(candidate);
        }
        // Existing artifacts stay where they are, so the folder they were
        // made in must stay trusted by the artifact escape guard.
        if previous != config.files_dir() && !config.files_dir_history.contains(&previous) {
            config.files_dir_history.push(previous);
        }
        config.save().await.map_err(|e| e.to_string())?;
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::AgentPathsChanged);
        log::debug!(
            "[config][agent_paths] files_dir now '{}'",
            config.files_dir().display()
        );
    }

    Ok(Outcome::new(agent_paths_payload(config), notes))
}

/// Loads the configuration, applies agent-paths updates, and saves it.
pub async fn load_and_apply_agent_paths_settings(
    update: AgentPathsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_agent_paths_settings(&mut config, update).await
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Reports the agent's filesystem roots so the UI can render them live
/// instead of hard-coding strings that drift away from `Config`.
pub async fn get_agent_paths() -> Result<Outcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    Ok(Outcome::new(
        agent_paths_payload(&config),
        vec![format!(
            "agent paths resolved (action={}, workspace={}, source={})",
            config.action_dir.display(),
            config.workspace_dir.display(),
            action_dir_source(&config),
        )],
    ))
}
