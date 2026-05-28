use crate::openhuman::config::Config;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

const DAEMON_STALE_SECONDS: i64 = 30;
const SCHEDULER_STALE_SECONDS: i64 = 120;
const CHANNEL_STALE_SECONDS: i64 = 300;
const COMMAND_VERSION_PREVIEW_CHARS: usize = 60;

// ── Diagnostic item ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticItem {
    pub severity: Severity,
    pub category: String,
    pub message: String,
}

impl DiagnosticItem {
    fn ok(category: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Ok,
            category: category.into(),
            message: msg.into(),
        }
    }
    fn warn(category: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warn,
            category: category.into(),
            message: msg.into(),
        }
    }
    fn error(category: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            category: category.into(),
            message: msg.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorSummary {
    pub ok: usize,
    pub warnings: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub items: Vec<DiagnosticItem>,
    pub summary: DoctorSummary,
}

// ── Public entry point ───────────────────────────────────────────

/// Build the full doctor report.
///
/// `ops::doctor_report` runs this in `tokio::task::spawn_blocking` because the
/// checks are synchronous and may touch the file system, sqlite, or local HTTP
/// endpoints. Keep this function blocking-only; add async probes in the caller
/// or behind their own runtime boundary instead of introducing `.await` here.
pub fn run(config: &Config) -> Result<DoctorReport> {
    let mut items: Vec<DiagnosticItem> = Vec::new();

    check_config_semantics(config, &mut items);
    check_workspace(config, &mut items);
    check_daemon_state(config, &mut items);
    check_environment(&mut items);
    check_memory_tree_db(config, &mut items);
    check_embedding_model_health(config, &mut items);
    check_claude_agent_sdk(config, &mut items);

    let errors = items
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = items
        .iter()
        .filter(|i| i.severity == Severity::Warn)
        .count();
    let ok = items.iter().filter(|i| i.severity == Severity::Ok).count();

    Ok(DoctorReport {
        items,
        summary: DoctorSummary {
            ok,
            warnings,
            errors,
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelProbeOutcome {
    Ok,
    Skipped,
    AuthOrAccess,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeEntry {
    pub provider: String,
    pub outcome: ModelProbeOutcome,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeSummary {
    pub ok: usize,
    pub skipped: usize,
    pub auth_or_access: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeReport {
    pub entries: Vec<ModelProbeEntry>,
    pub summary: ModelProbeSummary,
}

fn doctor_model_targets() -> Vec<String> {
    crate::openhuman::inference::provider::list_providers()
        .into_iter()
        .map(|provider| provider.name.to_string())
        .collect()
}

pub fn run_models(_config: &Config, _use_cache: bool) -> Result<ModelProbeReport> {
    let targets = doctor_model_targets();

    if targets.is_empty() {
        anyhow::bail!("No providers available for model probing");
    }

    let skipped_count = targets.len();
    let entries = targets
        .into_iter()
        .map(|provider| ModelProbeEntry {
            provider,
            outcome: ModelProbeOutcome::Skipped,
            message: Some("model catalog refresh removed".to_string()),
        })
        .collect();

    Ok(ModelProbeReport {
        entries,
        summary: ModelProbeSummary {
            ok: 0,
            skipped: skipped_count,
            auth_or_access: 0,
            errors: 0,
        },
    })
}

// ── Config semantic validation ───────────────────────────────────

fn check_config_semantics(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "config";

    // Config file exists
    if config.config_path.exists() {
        items.push(DiagnosticItem::ok(
            cat,
            format!("config file: {}", config.config_path.display()),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!("config file not found: {}", config.config_path.display()),
        ));
    }

    // Backend API URL
    if let Some(url) = config
        .api_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        items.push(DiagnosticItem::ok(cat, format!("api_url: {url}")));
    } else {
        let resolved = crate::api::config::effective_api_url(&config.api_url);
        items.push(DiagnosticItem::ok(
            cat,
            format!("api_url: (unset) resolved to {resolved}"),
        ));
    }

    match crate::api::jwt::get_session_token(config) {
        Ok(Some(token)) if !token.trim().is_empty() => {
            items.push(DiagnosticItem::ok(cat, "signed in with app session JWT"));
        }
        Ok(_) => {
            items.push(DiagnosticItem::warn(
                cat,
                "no app session JWT — not signed in",
            ));
        }
        Err(err) => {
            items.push(DiagnosticItem::error(
                cat,
                format!("failed to read app session JWT: {err}"),
            ));
        }
    }

    // Model configured
    if config.default_model.is_some() {
        items.push(DiagnosticItem::ok(
            cat,
            format!(
                "default model: {}",
                config.default_model.as_deref().unwrap_or("?")
            ),
        ));
    } else {
        items.push(DiagnosticItem::warn(cat, "no default_model configured"));
    }

    // Temperature range
    if config.default_temperature >= 0.0 && config.default_temperature <= 2.0 {
        items.push(DiagnosticItem::ok(
            cat,
            format!(
                "temperature {:.1} (valid range 0.0-2.0)",
                config.default_temperature
            ),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "temperature {:.1} is out of range (expected 0.0-2.0)",
                config.default_temperature
            ),
        ));
    }

    // Reliability: fallback providers (legacy; ignored at runtime)
    if !config.reliability.fallback_providers.is_empty() {
        items.push(DiagnosticItem::warn(
            cat,
            "reliability.fallback_providers is set but ignored (single backend)",
        ));
    }

    // Model routes validation
    for route in &config.model_routes {
        if route.hint.is_empty() {
            items.push(DiagnosticItem::warn(cat, "model route with empty hint"));
        }
        if route.model.is_empty() {
            items.push(DiagnosticItem::warn(
                cat,
                format!("model route \"{}\" has empty model", route.hint),
            ));
        }
    }

    // Embedding routes validation
    for route in &config.embedding_routes {
        if route.hint.trim().is_empty() {
            items.push(DiagnosticItem::warn(cat, "embedding route with empty hint"));
        }
        if let Some(reason) = embedding_provider_validation_error(&route.provider) {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "embedding route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
        }
        if route.model.trim().is_empty() {
            items.push(DiagnosticItem::warn(
                cat,
                format!("embedding route \"{}\" has empty model", route.hint),
            ));
        }
        if route.dimensions.is_some_and(|value| value == 0) {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "embedding route \"{}\" has invalid dimensions=0",
                    route.hint
                ),
            ));
        }
    }

    if let Some(hint) = config
        .memory
        .embedding_model
        .strip_prefix("hint:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !config
            .embedding_routes
            .iter()
            .any(|route| route.hint.trim() == hint)
        {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "memory.embedding_model uses hint \"{hint}\" but no matching [[embedding_routes]] entry exists"
                ),
            ));
        }
    }

    // Channel: at least one configured
    let cc = &config.channels_config;
    let has_channel = cc.telegram.is_some()
        || cc.discord.is_some()
        || cc.slack.is_some()
        || cc.imessage.is_some()
        || cc.matrix.is_some()
        || cc.whatsapp.is_some()
        || cc.email.is_some()
        || cc.irc.is_some()
        || cc.lark.is_some()
        || cc.webhook.is_some();

    if has_channel {
        items.push(DiagnosticItem::ok(cat, "at least one channel configured"));
    } else {
        items.push(DiagnosticItem::warn(
            cat,
            "no channels configured - configure one in the UI",
        ));
    }

    // Delegate agents
    let mut agent_names: Vec<_> = config.agents.keys().collect();
    agent_names.sort();
    for name in agent_names {
        let agent = config.agents.get(name).unwrap();
        if agent.model.trim().is_empty() {
            items.push(DiagnosticItem::warn(
                cat,
                format!("delegate agent \"{name}\" has empty model"),
            ));
        }
    }
}

fn embedding_provider_validation_error(name: &str) -> Option<String> {
    let normalized = name.trim();
    if normalized.eq_ignore_ascii_case("none") || normalized.eq_ignore_ascii_case("openai") {
        return None;
    }

    let Some(url) = normalized.strip_prefix("custom:") else {
        return Some("supported values: none, openai, custom:<url>".into());
    };

    let url = url.trim();
    if url.is_empty() {
        return Some("custom provider requires a non-empty URL after 'custom:'".into());
    }

    match reqwest::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => None,
        Ok(parsed) => Some(format!(
            "custom provider URL must use http/https, got '{}'",
            parsed.scheme()
        )),
        Err(err) => Some(format!("invalid custom provider URL: {err}")),
    }
}

// ── Workspace integrity ──────────────────────────────────────────

fn check_workspace(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "workspace";
    let ws = &config.workspace_dir;

    if ws.exists() {
        items.push(DiagnosticItem::ok(
            cat,
            format!("directory exists: {}", ws.display()),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!("directory missing: {}", ws.display()),
        ));
        return;
    }

    // Writable check
    let probe = workspace_probe_path(ws);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(mut probe_file) => {
            let write_result = probe_file.write_all(b"probe");
            drop(probe_file);
            let _ = std::fs::remove_file(&probe);
            match write_result {
                Ok(()) => items.push(DiagnosticItem::ok(cat, "directory is writable")),
                Err(e) => items.push(DiagnosticItem::error(
                    cat,
                    format!("directory write probe failed: {e}"),
                )),
            }
        }
        Err(e) => {
            items.push(DiagnosticItem::error(
                cat,
                format!("directory is not writable: {e}"),
            ));
        }
    }

    // Minimal workspace folders
    let mem_dir = ws.join("memory");
    if mem_dir.exists() {
        items.push(DiagnosticItem::ok(
            cat,
            format!("memory directory: {}", mem_dir.display()),
        ));
    } else {
        items.push(DiagnosticItem::warn(
            cat,
            format!("memory directory missing: {}", mem_dir.display()),
        ));
    }

    // Check for config templates or docs
    let prompt = ws.join("SYSTEM.md");
    if prompt.exists() {
        items.push(DiagnosticItem::ok(
            cat,
            format!("SYSTEM prompt: {}", prompt.display()),
        ));
    } else {
        items.push(DiagnosticItem::warn(
            cat,
            format!("SYSTEM prompt missing: {}", prompt.display()),
        ));
    }

    // Disk space warning (best-effort)
    if let Some(avail_mb) = available_disk_space_mb(ws) {
        if avail_mb < 512 {
            items.push(DiagnosticItem::warn(
                cat,
                format!("low disk space: {avail_mb} MB free"),
            ));
        } else {
            items.push(DiagnosticItem::ok(
                cat,
                format!("disk space OK: {avail_mb} MB free"),
            ));
        }
    }
}

fn available_disk_space_mb(path: &Path) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        return available_disk_space_mb_windows(path);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = std::process::Command::new("df")
            .arg("-m")
            .arg(path)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_df_available_mb(&stdout)
    }
}

#[cfg(not(target_os = "windows"))]
fn parse_df_available_mb(stdout: &str) -> Option<u64> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let avail = line.split_whitespace().nth(3)?;
    avail.parse::<u64>().ok()
}

#[cfg(target_os = "windows")]
fn available_disk_space_mb_windows(path: &Path) -> Option<u64> {
    use std::path::{Component, Prefix};

    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let letter = canonical.components().find_map(|c| match c {
        Component::Prefix(pc) => match pc.kind() {
            Prefix::Disk(b) | Prefix::VerbatimDisk(b) => Some((b as char).to_ascii_uppercase()),
            _ => None,
        },
        _ => None,
    })?;

    // PowerShell is ubiquitous on supported Windows; `Get-PSDrive` needs no admin
    // and returns free bytes as a single integer line.
    let script = format!("(Get-PSDrive -Name {letter} -ErrorAction Stop).Free");
    let mut cmd = std::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &script,
    ]);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let bytes: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(bytes / (1024 * 1024))
}

fn workspace_probe_path(workspace_dir: &Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    workspace_dir.join(format!(
        ".openhuman_doctor_probe_{}_{}",
        std::process::id(),
        nanos
    ))
}

// ── Daemon state ────────────────────────────────────────────────

fn check_daemon_state(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "daemon";
    let state_file = crate::openhuman::service::daemon::state_file_path(config);

    if !state_file.exists() {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "state file not found: {} - is the daemon running?",
                state_file.display()
            ),
        ));
        return;
    }

    let raw = match std::fs::read_to_string(&state_file) {
        Ok(r) => r,
        Err(e) => {
            items.push(DiagnosticItem::error(
                cat,
                format!("cannot read state file: {e}"),
            ));
            return;
        }
    };

    let snapshot: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            items.push(DiagnosticItem::error(
                cat,
                format!("invalid state JSON: {e}"),
            ));
            return;
        }
    };

    // Daemon heartbeat freshness
    let updated_at = snapshot
        .get("updated_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    if let Ok(ts) = DateTime::parse_from_rfc3339(updated_at) {
        let age = Utc::now()
            .signed_duration_since(ts.with_timezone(&Utc))
            .num_seconds();
        if age <= DAEMON_STALE_SECONDS {
            items.push(DiagnosticItem::ok(
                cat,
                format!("heartbeat fresh ({age}s ago)"),
            ));
        } else {
            items.push(DiagnosticItem::error(
                cat,
                format!("heartbeat stale ({age}s ago)"),
            ));
        }
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!("invalid daemon timestamp: {updated_at}"),
        ));
    }

    // Components
    if let Some(components) = snapshot
        .get("components")
        .and_then(serde_json::Value::as_object)
    {
        // Scheduler
        if let Some(scheduler) = components.get("scheduler") {
            let scheduler_ok = scheduler
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| s == "ok");
            let scheduler_age = scheduler
                .get("last_ok")
                .and_then(serde_json::Value::as_str)
                .and_then(parse_rfc3339)
                .map_or(i64::MAX, |dt| {
                    Utc::now().signed_duration_since(dt).num_seconds()
                });

            if scheduler_ok && scheduler_age <= SCHEDULER_STALE_SECONDS {
                items.push(DiagnosticItem::ok(
                    cat,
                    format!("scheduler healthy (last ok {scheduler_age}s ago)"),
                ));
            } else {
                items.push(DiagnosticItem::error(
                    cat,
                    format!("scheduler unhealthy (ok={scheduler_ok}, age={scheduler_age}s)"),
                ));
            }
        } else {
            items.push(DiagnosticItem::warn(
                cat,
                "scheduler component not tracked yet",
            ));
        }

        // Channels
        let mut channel_count = 0u32;
        let mut stale = 0u32;
        for (name, component) in components {
            if !name.starts_with("channel:") {
                continue;
            }
            channel_count += 1;
            let status_ok = component
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| s == "ok");
            let age = component
                .get("last_ok")
                .and_then(serde_json::Value::as_str)
                .and_then(parse_rfc3339)
                .map_or(i64::MAX, |dt| {
                    Utc::now().signed_duration_since(dt).num_seconds()
                });

            if status_ok && age <= CHANNEL_STALE_SECONDS {
                items.push(DiagnosticItem::ok(
                    cat,
                    format!("{name} fresh ({age}s ago)"),
                ));
            } else {
                stale += 1;
                items.push(DiagnosticItem::error(
                    cat,
                    format!("{name} stale (ok={status_ok}, age={age}s)"),
                ));
            }
        }

        if channel_count == 0 {
            items.push(DiagnosticItem::warn(
                cat,
                "no channel components tracked yet",
            ));
        } else if stale > 0 {
            items.push(DiagnosticItem::warn(
                cat,
                format!("{channel_count} channels, {stale} stale"),
            ));
        }
    }
}

// ── Environment checks ───────────────────────────────────────────

fn check_environment(items: &mut Vec<DiagnosticItem>) {
    let cat = "environment";

    // git
    check_command_available("git", &["--version"], cat, items);

    // Shell
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.is_empty() {
        items.push(DiagnosticItem::warn(cat, "$SHELL not set"));
    } else {
        items.push(DiagnosticItem::ok(cat, format!("shell: {shell}")));
    }

    // HOME
    if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
        items.push(DiagnosticItem::ok(cat, "home directory env set"));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            "neither $HOME nor $USERPROFILE is set",
        ));
    }

    // Optional tools
    check_command_available("curl", &["--version"], cat, items);
}

fn check_command_available(
    cmd: &str,
    args: &[&str],
    cat: &'static str,
    items: &mut Vec<DiagnosticItem>,
) {
    let mut child_cmd = std::process::Command::new(cmd);
    child_cmd
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match child_cmd.output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("(unknown)")
                .to_string();
            items.push(DiagnosticItem::ok(cat, format!("{cmd}: {version}")));
        }
        Ok(output) => {
            let preview = String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("(failed)")
                .to_string();
            items.push(DiagnosticItem::warn(
                cat,
                format!("{cmd} not available ({preview})"),
            ));
        }
        Err(err) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!("{cmd} not available ({err})"),
            ));
        }
    }
}

// ── Memory-tree DB health ────────────────────────────────────────

/// Probe the memory-tree SQLite database and push a [`DiagnosticItem`].
///
/// - If the DB directory / file does not exist yet: `Warn` (not yet created).
/// - If a stale `.db-shm` file is present alongside the DB: `Warn`.
/// - If we can open the DB and run a basic probe query: `Ok`.
/// - If the probe fails: `Error`.
fn check_memory_tree_db(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "memory_tree_db";
    let db_path = config.workspace_dir.join("memory_tree").join("chunks.db");

    // ── Stale side-files (checked even when chunks.db is absent) ────
    let base_name = db_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let shm = db_path.with_file_name(format!("{base_name}-shm"));
    let wal = db_path.with_file_name(format!("{base_name}-wal"));
    for sidecar in [&shm, &wal] {
        if sidecar.exists() {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "stale SQLite side-file present (may indicate unclean shutdown): {}",
                    sidecar.display()
                ),
            ));
        }
    }

    // ── File existence ──────────────────────────────────────────────
    if !db_path.exists() {
        items.push(DiagnosticItem::warn(
            cat,
            format!(
                "DB not yet created (first ingest will initialise it): {}",
                db_path.display()
            ),
        ));
        return;
    }

    // ── Probe connection ─────────────────────────────────────────────
    match crate::openhuman::memory_store::chunks::store::with_connection(config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_chunks", [], |r| r.get(0))?;
        Ok(n)
    }) {
        Ok(count) => {
            items.push(DiagnosticItem::ok(
                cat,
                format!("DB accessible at {} ({count} chunks)", db_path.display()),
            ));
        }
        Err(err) => {
            items.push(DiagnosticItem::error(
                cat,
                format!("DB probe failed at {}: {err:#}", db_path.display()),
            ));
        }
    }
}

// ── Embedding model health ───────────────────────────────────────

/// Probe the configured embedding provider and model.
///
/// - If the intended provider is not `"ollama"` (e.g. cloud): `Ok` — no
///   local daemon is involved and nothing to diagnose here.
/// - If Ollama is configured but the daemon at `<base_url>/api/tags` is
///   unreachable: `Error` with the pull command as the fix hint.
/// - If the daemon is reachable but the configured embedding model is not
///   listed in `/api/tags`: `Error` with `ollama pull <model>` guidance.
/// - If both daemon and model are healthy: `Ok`.
///
/// This check is synchronous (uses a small blocking HTTP call) so it fits
/// the existing `run()` contract. The timeout is capped at 3 s to avoid
/// stalling `openhuman doctor` on a very slow Ollama daemon.
fn check_embedding_model_health(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "embedding_model";

    // Resolve the effective (intended, non-probed) embedding settings.
    let local_embedding_model = config.workload_local_model("embeddings");
    let (provider, model, _dims) =
        crate::openhuman::memory_store::factories::effective_embedding_settings(
            &config.memory,
            local_embedding_model.as_deref(),
        );

    log::debug!("[doctor] check_embedding_model_health: provider={provider} model={model}");

    if provider != "ollama" {
        // Cloud or custom provider — no local daemon to probe.
        items.push(DiagnosticItem::ok(
            cat,
            format!("embedding provider: {provider} (model: {model}) — no local daemon required"),
        ));
        return;
    }

    // Ollama path: probe reachability then model availability.
    let base_url = crate::openhuman::inference::local::ollama_base_url();
    let tags_url = format!("{}/api/tags", base_url.trim_end_matches('/'));

    log::debug!("[doctor] probing ollama at {tags_url} for embedding model {model}");

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!("could not build HTTP client for Ollama probe: {e}"),
            ));
            return;
        }
    };

    let resp = match client.get(&tags_url).send() {
        Ok(r) => r,
        Err(e) => {
            items.push(DiagnosticItem::error(
                cat,
                format!(
                    "Ollama daemon unreachable at {base_url} — embedding model `{model}` cannot be used. \
                     Start Ollama, then run: ollama pull {model}  (error: {e})"
                ),
            ));
            return;
        }
    };

    if !resp.status().is_success() {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "Ollama /api/tags returned {} at {base_url} — cannot verify embedding model `{model}`. \
                 Start Ollama and run: ollama pull {model}",
                resp.status()
            ),
        ));
        return;
    }

    // Parse the tags response and look for the configured model.
    let body = match resp.text() {
        Ok(t) => t,
        Err(e) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!("Ollama /api/tags response could not be read: {e}"),
            ));
            return;
        }
    };

    // Parse the JSON and extract the `models` array.  If the response is
    // malformed or the schema changed (missing `models` key), report that
    // explicitly instead of falling through to "model NOT installed".
    let models_array = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => match v.get("models").and_then(|m| m.as_array()) {
            Some(arr) => arr.clone(),
            None => {
                items.push(DiagnosticItem::warn(
                    cat,
                    format!(
                        "Ollama /api/tags response is missing the `models` key — \
                         cannot verify embedding model `{model}`. Ollama API may have changed."
                    ),
                ));
                return;
            }
        },
        Err(e) => {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "Ollama /api/tags returned invalid JSON — \
                     cannot verify embedding model `{model}`: {e}"
                ),
            ));
            return;
        }
    };

    let model_found = models_array.iter().any(|entry| {
        entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| model_matches(name, &model))
            .unwrap_or(false)
    });

    if model_found {
        items.push(DiagnosticItem::ok(
            cat,
            format!("embedding model `{model}` is installed and reachable at {base_url}"),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "embedding model `{model}` is NOT installed on Ollama at {base_url}. \
                 Run: ollama pull {model}"
            ),
        ));
    }
}

// ── Claude Agent SDK check ───────────────────────────────────────

fn check_claude_agent_sdk(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let sdk = &config.claude_agent_sdk;
    if !sdk.enabled {
        return;
    }

    tracing::debug!("probe:claude_agent_sdk:entry binary={}", sdk.binary);

    // Probe the configured binary by running `<binary> --version`.
    let mut cmd = std::process::Command::new(&sdk.binary);
    cmd.arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    tracing::debug!(
        "probe:claude_agent_sdk:exec binary={} cmd=--version",
        sdk.binary
    );

    match cmd.output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("(unknown version)")
                .to_string();
            tracing::info!(
                "probe:claude_agent_sdk:ok binary={} version={}",
                sdk.binary,
                version
            );
            items.push(DiagnosticItem::ok(
                "claude_agent_sdk",
                format!("claude CLI found (binary='{}'): {version}", sdk.binary),
            ));
            tracing::debug!(
                "probe:claude_agent_sdk:exit binary={} result=ok",
                sdk.binary
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let preview = stderr.lines().next().unwrap_or("(no stderr)");
            tracing::warn!(
                "probe:claude_agent_sdk:warn binary={} status={:?} stderr={}",
                sdk.binary,
                output.status,
                truncate_for_display(preview, COMMAND_VERSION_PREVIEW_CHARS)
            );
            items.push(DiagnosticItem::warn(
                "claude_agent_sdk",
                format!(
                    "claude CLI execution failed (binary='{}', status={}). {}",
                    sdk.binary,
                    output.status,
                    truncate_for_display(preview, COMMAND_VERSION_PREVIEW_CHARS)
                ),
            ));
            tracing::debug!(
                "probe:claude_agent_sdk:exit binary={} result=warn",
                sdk.binary
            );
        }
        Err(err) => {
            tracing::warn!(
                "probe:claude_agent_sdk:warn binary={} err={}",
                sdk.binary,
                err
            );
            items.push(DiagnosticItem::warn(
                "claude_agent_sdk",
                format!(
                    "claude CLI not found or not executable (configured binary='{}'): {}. \
                     Install from https://claude.ai/code or set claude_agent_sdk.binary in config.",
                    sdk.binary, err
                ),
            ));
            tracing::debug!(
                "probe:claude_agent_sdk:exit binary={} result=warn",
                sdk.binary
            );
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────

fn parse_rfc3339(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn model_matches(installed: &str, configured: &str) -> bool {
    if installed == configured {
        return true;
    }

    if installed.contains(':') && configured.contains(':') {
        return false;
    }

    model_base(installed) == model_base(configured)
}

fn model_base(model: &str) -> &str {
    model.split(':').next().unwrap()
}

fn truncate_for_display(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }

    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_len {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
