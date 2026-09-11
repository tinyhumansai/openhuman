
// ── Daemon state ────────────────────────────────────────────────

fn check_daemon_state(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "daemon";
    let state_file = crate::openhuman::platform::service::daemon::state_file_path(config);

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

/// Probe the memory-tree and push [`DiagnosticItem`]s.
///
/// - If the legacy SQLite file does not exist: `Warn` (not yet created by the
///   embedded driver). This is an informational SQLite-artifact check, not a
///   gate: drivers that store memory elsewhere have no `chunks.db` by design.
/// - If a stale `.db-shm` file is present alongside the DB: `Warn`.
/// - If the driver answered with a chunk count: `Ok`.
/// - If it did not: `Error`.
///
/// The file checks are this function's own — they are `std::fs` calls about a
/// path, and a driver has nothing to say about them. The count is
/// `memory_chunks`, taken by the async caller: see [`MemoryChunkCount`] for
/// why it arrives as an argument rather than being read here.
///
/// The driver probe always runs regardless of file existence, so a bound driver
/// that does not use SQLite still surfaces its health here.
fn check_memory_tree_db(
    config: &Config,
    memory_chunks: &MemoryChunkCount,
    items: &mut Vec<DiagnosticItem>,
) {
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

    // ── SQLite-artifact check (informational only, not a gate) ──────
    if !db_path.exists() {
        items.push(DiagnosticItem::warn(
            cat,
            format!("legacy SQLite artifact is absent: {}", db_path.display()),
        ));
    }

    // ── Driver probe ────────────────────────────────────────────────
    // The count used to be a `SELECT COUNT(*) FROM mem_tree_chunks` through
    // the engine's own connection helper. It is the bound driver's answer now,
    // which is what lets this check mean something on a workspace whose memory
    // is not SQLite at all — and what takes the engine crate out of this file.
    match memory_chunks {
        Ok(count) => {
            log::debug!(
                "[doctor] check_memory_tree_db: driver reported {count} chunks at {}",
                db_path.display()
            );
            items.push(DiagnosticItem::ok(
                cat,
                format!(
                    "memory driver accessible ({count} chunks); SQLite artifact: {}",
                    db_path.display()
                ),
            ));
        }
        Err(err) => {
            log::debug!(
                "[doctor] check_memory_tree_db: chunk-count probe failed at {}: {err}",
                db_path.display()
            );
            items.push(DiagnosticItem::error(
                cat,
                format!("DB probe failed at {}: {err}", db_path.display()),
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
        crate::openhuman::inference::embeddings::effective_embedding_settings(
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
