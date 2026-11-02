//! Agent filesystem path configuration and directory helpers.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

/// Partial update for the agent's editable filesystem roots.
///
/// `action_dir` and the files folder are editable. `workspace_dir` and
/// `projects_dir` remain read-only.
#[derive(Debug, Clone, Default)]
pub struct AgentPathsPatch {
    /// New action sandbox root. `Some("")`/whitespace clears the override and
    /// reverts to the default; `Some(path)` sets it; `None` leaves it unchanged.
    pub action_dir: Option<String>,
    /// New folder for agent deliverables. Same `Some("")` / `Some(path)` /
    /// `None` semantics as `action_dir`. Affects new artifacts only.
    pub files_dir: Option<String>,
}

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
        crate::artifacts::migrate_legacy_artifacts(&config.workspace_dir, &files_dir).await;
    if report != crate::artifacts::MigrationReport::default() {
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
    if crate::security::SecurityPolicy::is_always_forbidden_async(&candidate).await {
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

pub(crate) fn canonical_prefix(path: &Path) -> PathBuf {
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

        config.save().await.map_err(super::config_save_error)?;

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
        config.save().await.map_err(super::config_save_error)?;
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
    ctx: &DispatchCtx,
    update: AgentPathsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
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
pub async fn get_agent_paths(ctx: &DispatchCtx) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
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
