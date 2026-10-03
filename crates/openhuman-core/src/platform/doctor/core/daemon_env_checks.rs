//! Daemon liveness, environment, and command-availability checks.

use chrono::{DateTime, Utc};

use crate::config::Config;

use super::types::DiagnosticItem;

const DAEMON_STALE_SECONDS: i64 = 30;
const SCHEDULER_STALE_SECONDS: i64 = 120;
const CHANNEL_STALE_SECONDS: i64 = 300;
pub(super) const COMMAND_VERSION_PREVIEW_CHARS: usize = 60;

pub(super) fn check_daemon_state(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "daemon";
    let state_file = crate::platform::service::daemon::state_file_path(config);

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

pub(super) fn check_environment(items: &mut Vec<DiagnosticItem>) {
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

pub(super) fn truncate_for_display(text: &str, max_len: usize) -> String {
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

fn parse_rfc3339(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
