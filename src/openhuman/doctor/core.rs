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

pub fn run(config: &Config) -> Result<DoctorReport> {
    let mut items: Vec<DiagnosticItem> = Vec::new();

    check_config_semantics(config, &mut items);
    check_workspace(config, &mut items);
    check_daemon_state(config, &mut items);
    check_environment(&mut items);

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
    crate::openhuman::providers::list_providers()
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
    if std::env::consts::OS == "windows" {
        let _ = path; // TODO: add a Windows-specific implementation
        return None;
    }

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

fn parse_df_available_mb(stdout: &str) -> Option<u64> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let avail = line.split_whitespace().nth(3)?;
    avail.parse::<u64>().ok()
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
    match std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
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

// ── Helpers ──────────────────────────────────────────────────────

fn parse_rfc3339(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
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
mod tests {
    use super::*;

    #[test]
    fn config_validation_warns_no_channels() {
        let config = Config::default();
        let mut items = vec![];
        check_config_semantics(&config, &mut items);
        let ch_item = items.iter().find(|i| i.message.contains("channel"));
        assert!(ch_item.is_some());
        assert_eq!(ch_item.unwrap().severity, Severity::Warn);
    }

    #[test]
    fn truncate_for_display_short() {
        let s = "hello";
        assert_eq!(truncate_for_display(s, 10), s);
    }

    #[test]
    fn truncate_for_display_long() {
        let s = "abcdefghijklmnopqrstuvwxyz";
        let truncated = truncate_for_display(s, 5);
        assert!(truncated.starts_with("abcde"));
        assert!(truncated.ends_with("..."));
    }
}
