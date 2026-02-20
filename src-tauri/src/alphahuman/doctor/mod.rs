//! Diagnostic checks for Alphahuman configuration, workspace health, and daemon state.

use crate::alphahuman::config::Config;
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
        summary: DoctorSummary { ok, warnings, errors },
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

fn classify_model_probe_error(err_message: &str) -> ModelProbeOutcome {
    let lower = err_message.to_lowercase();

    if lower.contains("does not support live model discovery") {
        return ModelProbeOutcome::Skipped;
    }

    if [
        "401",
        "403",
        "429",
        "unauthorized",
        "forbidden",
        "api key",
        "token",
        "insufficient balance",
        "insufficient quota",
        "plan does not include",
        "rate limit",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
    {
        return ModelProbeOutcome::AuthOrAccess;
    }

    ModelProbeOutcome::Error
}

fn doctor_model_targets(provider_override: Option<&str>) -> Vec<String> {
    if let Some(provider) = provider_override.map(str::trim).filter(|p| !p.is_empty()) {
        return vec![provider.to_string()];
    }

    crate::alphahuman::providers::list_providers()
        .into_iter()
        .map(|provider| provider.name.to_string())
        .collect()
}

pub fn run_models(
    config: &Config,
    provider_override: Option<&str>,
    use_cache: bool,
) -> Result<ModelProbeReport> {
    let targets = doctor_model_targets(provider_override);

    if targets.is_empty() {
        anyhow::bail!("No providers available for model probing");
    }

    let mut entries = Vec::new();
    let mut ok_count = 0usize;
    let mut skipped_count = 0usize;
    let mut auth_count = 0usize;
    let mut error_count = 0usize;

    for provider_name in &targets {
        match crate::alphahuman::onboard::run_models_refresh(config, Some(provider_name), !use_cache)
        {
            Ok(_) => {
                ok_count += 1;
                entries.push(ModelProbeEntry {
                    provider: provider_name.clone(),
                    outcome: ModelProbeOutcome::Ok,
                    message: None,
                });
            }
            Err(error) => {
                let error_text = format_error_chain(&error);
                let outcome = classify_model_probe_error(&error_text);
                match outcome {
                    ModelProbeOutcome::Skipped => skipped_count += 1,
                    ModelProbeOutcome::AuthOrAccess => auth_count += 1,
                    ModelProbeOutcome::Error => error_count += 1,
                    ModelProbeOutcome::Ok => ok_count += 1,
                }
                entries.push(ModelProbeEntry {
                    provider: provider_name.clone(),
                    outcome,
                    message: Some(truncate_for_display(&error_text, 160)),
                });
            }
        }
    }

    if provider_override.is_some() && ok_count == 0 {
        anyhow::bail!("Model probe failed for target provider");
    }

    Ok(ModelProbeReport {
        entries,
        summary: ModelProbeSummary {
            ok: ok_count,
            skipped: skipped_count,
            auth_or_access: auth_count,
            errors: error_count,
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

    // Provider validity
    if let Some(ref provider) = config.default_provider {
        if let Some(reason) = provider_validation_error(provider) {
            items.push(DiagnosticItem::error(
                cat,
                format!("default provider \"{provider}\" is invalid: {reason}"),
            ));
        } else {
            items.push(DiagnosticItem::ok(
                cat,
                format!("provider \"{provider}\" is valid"),
            ));
        }
    } else {
        items.push(DiagnosticItem::error(cat, "no default_provider configured"));
    }

    // API key presence
    if config.default_provider.as_deref() != Some("ollama") {
        if config.api_key.is_some() {
            items.push(DiagnosticItem::ok(cat, "API key configured"));
        } else {
            items.push(DiagnosticItem::warn(
                cat,
                "no api_key set (may rely on env vars or provider defaults)",
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

    // Gateway port range
    let port = config.gateway.port;
    if port > 0 {
        items.push(DiagnosticItem::ok(cat, format!("gateway port: {port}")));
    } else {
        items.push(DiagnosticItem::error(cat, "gateway port is 0 (invalid)"));
    }

    // Reliability: fallback providers
    for fb in &config.reliability.fallback_providers {
        if let Some(reason) = provider_validation_error(fb) {
            items.push(DiagnosticItem::warn(
                cat,
                format!("fallback provider \"{fb}\" is invalid: {reason}"),
            ));
        }
    }

    // Model routes validation
    for route in &config.model_routes {
        if route.hint.is_empty() {
            items.push(DiagnosticItem::warn(cat, "model route with empty hint"));
        }
        if let Some(reason) = provider_validation_error(&route.provider) {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "model route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
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

    // Delegate agents: provider validity
    let mut agent_names: Vec<_> = config.agents.keys().collect();
    agent_names.sort();
    for name in agent_names {
        let agent = config.agents.get(name).unwrap();
        if let Some(reason) = provider_validation_error(&agent.provider) {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "agent \"{name}\" uses invalid provider \"{}\": {}",
                    agent.provider, reason
                ),
            ));
        }
    }
}

fn provider_validation_error(name: &str) -> Option<String> {
    match crate::alphahuman::providers::create_provider(name, None) {
        Ok(_) => None,
        Err(err) => Some(
            err.to_string()
                .lines()
                .next()
                .unwrap_or("invalid provider")
                .into(),
        ),
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
        let _ = path;
        return None;
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
        ".alphahuman_doctor_probe_{}_{}",
        std::process::id(),
        nanos
    ))
}

// ── Daemon state ────────────────────────────────────────────────

fn check_daemon_state(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "daemon";
    let state_file = crate::alphahuman::daemon::state_file_path(config);

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
            items.push(DiagnosticItem::error(cat, format!("cannot read state file: {e}")));
            return;
        }
    };

    let snapshot: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            items.push(DiagnosticItem::error(cat, format!("invalid state JSON: {e}")));
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
            items.push(DiagnosticItem::ok(cat, format!("heartbeat fresh ({age}s ago)")));
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
            items.push(DiagnosticItem::warn(cat, "scheduler component not tracked yet"));
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
                items.push(DiagnosticItem::ok(cat, format!("{name} fresh ({age}s ago)")));
            } else {
                stale += 1;
                items.push(DiagnosticItem::error(
                    cat,
                    format!("{name} stale (ok={status_ok}, age={age}s)"),
                ));
            }
        }

        if channel_count == 0 {
            items.push(DiagnosticItem::warn(cat, "no channel components tracked yet"));
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

fn check_command_available(cmd: &str, args: &[&str], cat: &'static str, items: &mut Vec<DiagnosticItem>) {
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

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut out = err.to_string();
    let mut cursor = err.source();

    while let Some(source) = cursor {
        out.push_str(": ");
        out.push_str(&source.to_string());
        cursor = source.source();
    }

    out
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
    fn provider_validation_detects_invalid() {
        let reason = provider_validation_error("imaginary");
        assert!(reason.is_some());
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
