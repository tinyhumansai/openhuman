
/// Recursive byte-count of files under `root`. Returns `0` when the root
/// does not exist or any traversal error occurs (best-effort; the status
/// panel is a UI convenience, not an audit surface).
fn compute_dir_size_bytes(root: &std::path::Path) -> u64 {
    if !root.exists() {
        return 0;
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        match entry {
            Ok(e) if e.file_type().is_file() => {
                if let Ok(meta) = e.metadata() {
                    total = total.saturating_add(meta.len());
                }
            }
            Ok(_) => {}
            Err(err) => {
                // Both `err.path()` and `walkdir::Error`'s `Display` impl
                // embed the absolute on-disk path (which lives under the
                // user's home directory), so we redact: log only whether a
                // path was attached and the underlying `io::ErrorKind`.
                // That's enough for diagnosis while keeping the user's
                // workspace layout out of the log file.
                log::warn!(
                    "[memory-tree][rpc] pipeline_status: dir walk error has_path={} kind={:?}",
                    err.path().is_some(),
                    err.io_error().map(|e| e.kind())
                );
            }
        }
    }
    total
}

/// #5324: how long the queue may hold eligible work without settling a single
/// job before the pipeline is reported as `degraded` rather than
/// `running`/`idle`.
///
/// A working pipeline settles jobs continuously, so this is idle time, not
/// backlog depth — a deep-but-draining backfill never approaches it. Six hours
/// is far outside a normal flush window (minutes) yet well inside the "broken
/// for a month" window the issue describes, so it cannot fire on a busy
/// machine or a laptop that was asleep for an hour.
pub(crate) const QUEUE_STALL_THRESHOLD_MS: i64 = 6 * 60 * 60 * 1000;

/// The #5324 stall verdict on its own: eligible work has been waiting for at
/// least [`QUEUE_STALL_THRESHOLD_MS`] without any job settling. Shared by the
/// status precedence below and by the `queue_stalled` flag the response
/// carries, so the two cannot disagree (openhuman#6025 review).
pub(crate) fn queue_is_stalled(queue_idle_ms: Option<i64>) -> bool {
    queue_idle_ms.is_some_and(|idle| idle >= QUEUE_STALL_THRESHOLD_MS)
}

/// Pure derivation of `(status, reason)` from raw signals. Split out so the
/// unit tests can exercise the precedence rules without spinning up a
/// store.
///
/// `queue_idle_ms` is how long the queue has held eligible work without
/// settling any job, or `None` when no eligible work is waiting (or the
/// metric could not be read).
fn derive_pipeline_status(
    is_paused: bool,
    mode: tinymemory_api::host::SchedulerGateMode,
    is_syncing: bool,
    failed: u64,
    failed_unrecoverable: u64,
    total_chunks: u64,
    degraded: &crate::openhuman::memory::tree::health::DegradedState,
    queue_idle_ms: Option<i64>,
) -> (String, Option<String>) {
    if is_paused {
        return (
            "paused".to_string(),
            Some(format!("scheduler gate mode = {}", mode.as_str())),
        );
    }
    // Host storage is unusable (EIO/ENOSPC/EROFS on the memory_tree path). This
    // is a foundational, unrecoverable error — the DB can't even open, so it
    // outranks the per-content recall/structure degradation below AND fires
    // regardless of `total_chunks` (on a dead disk we may not be able to count
    // chunks at all). Only the user can fix it (reseat/replace/free storage);
    // the actionable remediation text rides the `StorageUnavailable`
    // remediation key surfaced by the doctor's `first_blocking_cause`.
    if degraded.storage {
        return (
            "error".to_string(),
            Some("memory storage unavailable — check your disk / SD card".to_string()),
        );
    }
    // #3365: split the failed bucket by class. Only an UNRECOVERABLE failure
    // (budget / auth / dim-mismatch) is a hard `error` the user must act on —
    // it stays parked and can't self-heal. Transient failures are auto-requeued
    // by `requeue_transient_failed`, so they must NOT escalate to `error`; they
    // fall through to `degraded` ("failed, retrying") below. This fixes the prior
    // `failed > 0 → error` that flashed a scary error for a job about to retry.
    if failed_unrecoverable > 0 {
        return (
            "error".to_string(),
            Some(format!(
                "{failed_unrecoverable} unrecoverable failure(s) need action"
            )),
        );
    }
    // #5324: the queue is accepting work but not draining it. This is the
    // "silently broken for a month" shape — new files keep getting detected
    // and queued, health checks keep reporting `ok` because the process is
    // alive, and nothing ever becomes searchable memory. Liveness is not
    // output, so a queue whose oldest ready job has been waiting past the
    // threshold reports `degraded`, never `running`/`idle`.
    //
    // Sits below `error` (a typed unrecoverable failure is the more specific
    // diagnosis and carries its own remediation) and above the recall/structure
    // degradation, and is deliberately NOT gated on `total_chunks` — a queue
    // that never drained has no chunks to gate on, which is exactly the case
    // that must not read as `idle`.
    if queue_is_stalled(queue_idle_ms) {
        let hours = queue_idle_ms.unwrap_or(0) / (60 * 60 * 1000);
        return (
            "degraded".to_string(),
            Some(format!(
                "queue has not completed any job in {hours}h — memory is not growing"
            )),
        );
    }
    // #002 (FR-005): "degraded" sits below error but above syncing/running —
    // the pipeline is making progress, but recall/structure is reduced (or some
    // jobs failed transiently and are retrying) and the user should be told why.
    // Beats syncing/running so a half-working sync isn't reported as plain
    // "running"/"syncing".
    //
    // Only fires when there are chunks: degraded recall/structure is only
    // meaningful when there's actual content affected. An empty workspace with
    // a misconfigured embedder should show "idle" (nothing to recall) rather
    // than "degraded" (recall is broken for existing content).
    //
    // `failed` here is transient-only — any unrecoverable failure returned
    // `error` above, so a non-zero `failed` at this point means jobs that will
    // be auto-requeued.
    if (degraded.is_degraded() || failed > 0) && total_chunks > 0 {
        let mut parts: Vec<String> = Vec::new();
        if degraded.semantic_recall {
            parts.push("semantic recall disabled".to_string());
        }
        if degraded.structure {
            parts.push("wiki structure incomplete".to_string());
        }
        if failed > 0 {
            parts.push(format!("{failed} job(s) failed, retrying"));
        }
        return ("degraded".to_string(), Some(parts.join("; ")));
    }
    if is_syncing {
        return ("syncing".to_string(), None);
    }
    if total_chunks > 0 {
        return ("running".to_string(), None);
    }
    ("idle".to_string(), None)
}

/// Request shape for `memory_tree_set_enabled`. Single field — the caller
/// asks to enable (auto-mode) or pause (off-mode) all LLM-bound background
/// work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetEnabledRequest {
    /// `true` ⇒ scheduler-gate mode becomes `auto`. `false` ⇒ `off`.
    pub enabled: bool,
}

/// Response shape for `memory_tree_set_enabled`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetEnabledResponse {
    /// Echo of the requested `enabled` state (post-write).
    pub enabled: bool,
    /// `true` when the saved mode actually flipped; `false` for no-ops.
    pub changed: bool,
    /// New scheduler-gate mode as wire string (`auto` / `off`).
    pub mode: String,
}

/// `memory_tree_set_enabled` RPC handler (#1856 Part 1).
///
/// Flips `config.scheduler_gate().mode` to either `Auto` (enabled) or `Off`
/// (paused), persists to disk via `config.save()`, and hot-reloads the
/// live scheduler-gate state so any in-flight workers immediately observe
/// the new policy at their next `wait_for_capacity()` await.
///
/// Notes:
/// - This is intentionally a single-field RPC (no batched
///   `MemoryTreeSettingsPatch`) — keeps the surface tight while #1856
///   Part 2 work lands the broader settings story.
/// - The 20-min Composio fetch loop is *not* paused by this toggle yet —
///   that requires a separate `Notify` signal and is queued for Part 2.
pub async fn set_enabled_rpc(
    config: &mut Config,
    req: SetEnabledRequest,
) -> Result<RpcOutcome<SetEnabledResponse>, String> {
    use tinymemory_api::host::SchedulerGateMode;

    let prev_mode = config.scheduler_gate.mode;
    let new_mode = if req.enabled {
        SchedulerGateMode::Auto
    } else {
        SchedulerGateMode::Off
    };

    log::debug!(
        "[memory-tree][rpc] set_enabled: requested enabled={} prev_mode={} new_mode={}",
        req.enabled,
        prev_mode.as_str(),
        new_mode.as_str(),
    );

    if prev_mode == new_mode {
        log::info!(
            "[memory-tree][rpc] set_enabled: no-op (mode already {})",
            new_mode.as_str()
        );
        return Ok(RpcOutcome::single_log(
            SetEnabledResponse {
                enabled: req.enabled,
                changed: false,
                mode: new_mode.as_str().to_string(),
            },
            format!(
                "memory_tree: set_enabled no-op enabled={} mode={}",
                req.enabled,
                new_mode.as_str()
            ),
        ));
    }

    config.scheduler_gate.mode = new_mode;
    config.save().await.map_err(|e| {
        let msg = format!("set_enabled: config.save failed: {e}");
        log::warn!("[memory-tree][rpc] {msg}");
        msg
    })?;

    // Hot-reload the live gate state — workers re-poll inside
    // `wait_for_capacity` and pick up the new policy without a restart.
    crate::openhuman::cron::scheduler_gate::gate::update_config(config.scheduler_gate.clone());

    log::info!(
        "[memory-tree][rpc] set_enabled: scheduler_gate.mode {} -> {} (enabled={})",
        prev_mode.as_str(),
        new_mode.as_str(),
        req.enabled,
    );

    Ok(RpcOutcome::single_log(
        SetEnabledResponse {
            enabled: req.enabled,
            changed: true,
            mode: new_mode.as_str().to_string(),
        },
        format!(
            "memory_tree: set_enabled enabled={} mode={} changed=true",
            req.enabled,
            new_mode.as_str()
        ),
    ))
}
