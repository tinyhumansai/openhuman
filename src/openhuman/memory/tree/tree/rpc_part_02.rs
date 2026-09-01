
/// Aggregate counts over the bound driver's stored chunks.
///
/// Zeroed rather than refused when the driver does not serve `Maintenance`:
/// this feeds a status surface, and a status surface that errors tells the
/// user less than one reporting an empty store.
async fn store_stats(
    config: &Config,
) -> Result<crate::openhuman::memory::api::provider::types::StoreStats, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory-tree][rpc] store_stats: driver '{}' does not serve Maintenance; reporting empty",
            binding.driver_id()
        );
        return Ok(Default::default());
    };
    maintenance
        .store_stats()
        .await
        .map_err(|e| format!("store_stats: {e}"))
}

/// The bound driver's queue state, optionally narrowed to one job kind.
///
/// A driver error propagates, the same way [`store_stats`] propagates its own.
/// That matters most for `backfill_status_rpc`, which is asked whether a modal
/// may close: guessing "nothing pending" would dismiss it over a live
/// backfill. A driver that does not serve `Maintenance` still reports empty
/// rather than erroring, because it has no queue to be behind on.
async fn queue_stats(
    config: &Config,
    kind: Option<&str>,
) -> Result<crate::openhuman::memory::api::provider::types::QueueStats, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory-tree][rpc] queue_stats: driver '{}' does not serve Maintenance; reporting empty",
            binding.driver_id()
        );
        return Ok(Default::default());
    };
    maintenance
        .queue_stats(kind)
        .await
        .map_err(|e| format!("queue_stats: {e}"))
}

/// Whether the driver has a re-embed backfill chain running.
///
/// Scoped to the driver's process, not to this store — the contract member says
/// so in its own signature, which is why it is a member rather than a field on
/// [`queue_stats`]. A driver serving several stores answers the same for all of
/// them.
///
/// A driver without Maintenance reports `false` rather than erroring, matching
/// its siblings: "this driver runs no backfill" is true of it, not a fault the
/// caller can act on.
async fn backfill_in_progress(config: &Config) -> Result<bool, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory-tree][rpc] backfill_in_progress: driver '{}' does not serve Maintenance; reporting false",
            binding.driver_id()
        );
        return Ok(false);
    };
    maintenance
        .backfill_in_progress()
        .await
        .map_err(|e| format!("backfill_in_progress: {e}"))
}

/// Response from the `memory_backfill_status` RPC (#1574 §4b). The frontend
/// polls this while the re-embed modal is open to surface progress and to
/// dismiss the modal once the new embedding space is fully covered.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackfillStatusResponse {
    /// True while a re-embed backfill chain still has work pending — the
    /// #1365 flag OR a queued/running `reembed_backfill` job.
    pub in_progress: bool,
    /// Count of `reembed_backfill` jobs in `ready` or `running` state. `0`
    /// with `in_progress=false` means the active embedding space is fully
    /// covered (modal can close).
    pub pending_jobs: u64,
}

/// `memory_backfill_status` RPC handler (#1574 §4b). No inputs — reports
/// whether a per-model re-embed backfill is in flight so the UI can warn
/// the user that semantic recall is reduced until it drains.
pub async fn backfill_status_rpc(
    config: &Config,
) -> Result<RpcOutcome<BackfillStatusResponse>, String> {
    log::debug!("[memory::rpc] backfill_status: entry");
    // Asked of the bound driver rather than of TinyCortex's tables. No
    // `spawn_blocking` here any more: the driver owns whether its own reads
    // block, and a host that wraps them a second time is guessing about
    // storage it no longer talks to.
    let queue = queue_stats(config, Some(REEMBED_BACKFILL_KIND))
        .await
        .map_err(|e| {
            let msg = format!("memory_backfill_status: {e}");
            log::debug!("[memory::rpc] backfill_status: error: {msg}");
            msg
        })?;
    // Ready + running, not `total - done`: a failed backfill job is finished
    // with, and counting it as pending leaves the modal open forever.
    let pending_jobs: u64 = queue.ready + queue.running;
    // Asked of the driver, not of the host-linked engine's process-global. That
    // static covers the instant between one backfill link settling and the next
    // being enqueued, which the counts cannot see — but re-embedding runs in the
    // module now, and a `cdylib` has its own statics, so the host-side copy reads
    // `false` forever and the modal closes while work is still being prepared.
    //
    // The member is process-wide rather than store-scoped, and says so in its
    // signature; that is why it is not a `QueueStats` field, where a per-store
    // snapshot would have implied a scoping it does not have. A read failure
    // degrades to the counts rather than failing the polled RPC.
    let driver_backfilling = backfill_in_progress(config).await.unwrap_or_else(|e| {
        log::warn!("[memory::rpc] backfill_status: backfill_in_progress read failed: {e}");
        false
    });
    let in_progress = driver_backfilling || pending_jobs > 0;
    Ok(RpcOutcome::single_log(
        BackfillStatusResponse {
            in_progress,
            pending_jobs,
        },
        format!("memory_tree: backfill_status in_progress={in_progress} pending={pending_jobs}"),
    ))
}

// ── pipeline_status / set_enabled (#1856 Part 1) ─────────────────────────

/// Per-status counters for the `mem_tree_jobs` table — snapshot returned by
/// the `memory_tree_pipeline_status` RPC. Only the three states the status
/// panel surfaces are exposed; `done` / `cancelled` are intentionally
/// omitted to keep the wire payload small.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineJobCounts {
    /// Jobs queued and waiting for a worker (`status = 'ready'`).
    pub ready: u64,
    /// Jobs currently being processed by a worker (`status = 'running'`).
    pub running: u64,
    /// Jobs that exhausted retries and remain in the table for diagnosis
    /// (`status = 'failed'`).
    pub failed: u64,
}

/// Response from the `memory_tree_pipeline_status` RPC (#1856 Part 1).
///
/// Aggregates "is the Memory Tree healthy?" signals into a single payload
/// the UI status panel can render without secondary fetches:
///
/// - `status` is a coarse, UI-shaped string (`running`/`paused`/`syncing`/
///   `error`/`idle`) derived from the other fields so the frontend stays
///   purely presentational.
/// - `wiki_size_bytes` is a recursive walk of the on-disk `wiki/` sub-tree
///   under the memory-tree content root; recomputed every call (cheap for
///   typical workspaces). The walk is scoped to `wiki/` so the figure
///   reflects the user-visible wiki only — not the sibling `raw/`,
///   `email/`, `chat/`, `document/` staging directories.
/// - `pipeline_jobs` is a snapshot of the queue — running > 0 implies
///   active sync, failed > 0 implies degraded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineStatusResponse {
    /// Aggregated status string: `running` | `paused` | `syncing` |
    /// `degraded` | `error` | `idle`. Derivation:
    /// 1. `is_paused` (scheduler-gate `off`) wins → `paused`.
    /// 2. otherwise failed > 0 → `error`.
    /// 3. otherwise degraded (#002, recall/structure reduced) → `degraded`.
    /// 4. otherwise running > 0 → `syncing`.
    /// 5. otherwise total_chunks > 0 → `running`.
    /// 6. otherwise → `idle`.
    pub status: String,
    /// Optional human-readable reason — populated when status is
    /// `paused` or `error`. `None` otherwise.
    pub reason: Option<String>,
    /// Epoch milliseconds of the most-recent chunk timestamp across all
    /// sources. Zero when the store is empty.
    pub last_sync_ms: i64,
    /// Total `mem_tree_chunks` rows across all sources.
    pub total_chunks: u64,
    /// Recursive byte size of the on-disk `wiki/` sub-tree under the
    /// memory-tree content root. Zero when the `wiki/` directory does not
    /// exist yet or cannot be read. Scoped to `wiki/` so the value matches
    /// the user-visible "Wiki size" tile (#1856 follow-up).
    pub wiki_size_bytes: u64,
    /// Snapshot counts from `mem_tree_jobs`.
    pub pipeline_jobs: PipelineJobCounts,
    /// Convenience flag: at least one job is currently `running`.
    pub is_syncing: bool,
    /// Convenience flag: scheduler-gate is in `off` mode, so all LLM-bound
    /// background work is paused cooperatively.
    pub is_paused: bool,
    /// #002 (FR-002/FR-004): "the pipeline ran but output quality is reduced"
    /// — `semantic_recall` true when embeddings were skipped (no usable
    /// provider, so recall falls back to recency), `structure` true when
    /// extraction yielded nothing across the board (empty wiki). Carries the
    /// typed `cause` so the UI can render an actionable remediation. Additive:
    /// `#[serde(default)]` keeps older clients deserialising the response.
    #[serde(default)]
    pub degraded: crate::openhuman::memory::tree::health::DegradedState,
    /// #002 (FR-004): the single first blocking/most-significant cause, as a
    /// typed failure with an i18n remediation key. Populated from a failed
    /// job's classified reason or the active degradation cause; `None` when
    /// the pipeline is healthy. The frontend renders this verbatim (resolving
    /// `remediation_key`) instead of re-deriving a cause from raw counters.
    #[serde(default)]
    pub first_blocking_cause: Option<crate::openhuman::memory::tree::health::PipelineFailure>,
    /// #002 (FR-010 / US5): fraction of chunks with ≥1 indexed entity, in
    /// `[0.0, 1.0]`. Near 0 with `total_chunks > 0` means extraction is
    /// producing no structure (the "empty-but-built wiki"). `None` when the
    /// metric could not be measured (DB read error) — deliberately distinct
    /// from a genuine `Some(0.0)` so the status surface never misreports a
    /// broken measurement path as a structure failure. Additive
    /// (`#[serde(default)]` → `None` for older clients).
    #[serde(default)]
    pub extraction_coverage: Option<f32>,
    /// openhuman#5820: the most recent corrupt-store quarantine in this
    /// workspace, derived from disk (`memory_tree/chunks.db.corrupt-<ts>`),
    /// so it survives restarts and reaches a renderer that was not connected
    /// when the quarantine happened. `None` when nothing was ever quarantined.
    /// Reported until the rebuilt store holds a chunk again (`resynced`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantine: Option<QuarantineStatus>,
}

/// `memory_tree_pipeline_status` RPC handler (#1856 Part 1).
///
/// A corrupt-store quarantine as the status surface reports it (openhuman#5820).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineStatus {
    /// Epoch milliseconds of the quarantine, parsed from the file name's UTC
    /// timestamp (`chunks.db.corrupt-%Y%m%dT%H%M%SZ`).
    pub quarantined_at_ms: i64,
    /// The preserved copy of the damaged database. Local to this machine and
    /// shown only to its own user, so the user can hand it to recovery tooling.
    pub quarantined_path: String,
    /// Whether the rebuilt store holds any chunk again. The quarantine leaves
    /// an empty schema, so a non-empty store means the user has re-synced
    /// and the notice can retire. Deliberately not a timestamp comparison:
    /// chunk timestamps are *content* time (a mail's `sent_at`, a file's
    /// `modified_at`), so restored history predates the quarantine forever.
    pub resynced: bool,
}

/// Newest `chunks.db.corrupt-<ts>` under `<workspace>/memory_tree`, if any.
///
/// Disk is the durable record: the engine's quarantine renames the damaged
/// file beside the store and never deletes it, so a status read after a
/// restart — or from a renderer that missed the live event — still finds it.
/// Side-file quarantines (`chunks.db-wal.corrupt-…`) do not match the prefix.
fn latest_quarantine(
    workspace_dir: &std::path::Path,
    total_chunks: u64,
) -> Option<QuarantineStatus> {
    const PREFIX: &str = "chunks.db.corrupt-";
    let dir = workspace_dir.join("memory_tree");
    let newest = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let stamp = name.strip_prefix(PREFIX)?.to_owned();
            let at = chrono::NaiveDateTime::parse_from_str(&stamp, "%Y%m%dT%H%M%SZ").ok()?;
            Some((at.and_utc().timestamp_millis(), entry.path()))
        })
        .max_by_key(|(at, _)| *at)?;
    let (quarantined_at_ms, path) = newest;
    Some(QuarantineStatus {
        quarantined_at_ms,
        quarantined_path: path.display().to_string(),
        resynced: total_chunks > 0,
    })
}

/// Aggregates `list_sources` + `count_by_status` + a recursive disk-size
/// probe into the [`PipelineStatusResponse`] the UI status panel renders.
/// All blocking work is dispatched onto `spawn_blocking` so the async
/// runtime isn't held during SQLite or filesystem I/O.
pub async fn pipeline_status_rpc(
    config: &Config,
) -> Result<RpcOutcome<PipelineStatusResponse>, String> {
    use tinymemory_api::host::SchedulerGateMode;

    log::debug!("[memory-tree][rpc] pipeline_status: entry");

    // Chunk aggregates — count, extracted count and newest timestamp, in one
    // observation of the driver. Splitting them is what let a write land
    // between the count and the extracted count and report an extraction
    // coverage above 100%.
    let store = store_stats(config).await.map_err(|e| {
        log::warn!("[memory-tree][rpc] pipeline_status: {e}");
        e
    })?;
    let total_chunks = store.chunks;
    // The wire field is a plain `i64` where the driver answers `Option`, and
    // `0` is its established "never synced" value — an empty store has no
    // newest chunk, which is not a chunk stamped at the epoch.
    let last_sync_ms = store.most_recent_chunk_ms.unwrap_or(0).max(0);

    // Job counters — one observation of the queue, where this used to be five
    // separate reads at five instants. That mattered: `failed_unrecoverable`
    // is the #3365 left-right split (of the failed jobs, how many are the
    // hard, user-actionable kind vs transient ones that self-heal via
    // auto-requeue, since only the former escalates to `error`), and a retry
    // landing between the two reads could report more unrecoverable failures
    // than failures.
    //
    // #5324's stall signal comes from the same snapshot for the same reason —
    // an idle time computed against counts taken at a different instant reads
    // as a stall that never happened.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let queue = queue_stats(config, None).await.map_err(|e| {
        log::warn!("[memory-tree][rpc] pipeline_status: {e}");
        e
    })?;
    let pipeline_jobs = PipelineJobCounts {
        ready: queue.ready,
        running: queue.running,
        failed: queue.failed,
    };
    let failed_unrecoverable = queue.failed_unrecoverable;
    let queue_idle_ms = queue_idle_ms(&queue, now_ms);

    // Disk size — best-effort. Permission errors etc. degrade to 0 with a
    // warn log rather than failing the whole RPC. Scoped to the `wiki/`
    // sub-directory so the tile lives up to its "Wiki size" label — the
    // sibling `raw/` / `email/` / `chat/` / `document/` staging directories
    // hold pre-canonicalised content and should not roll into the figure
    // surfaced to the user (#1856 CodeRabbit feedback).
    let wiki_root = config.memory_tree_content_root().join("wiki");
    let wiki_size_bytes = tokio::task::spawn_blocking(move || compute_dir_size_bytes(&wiki_root))
        .await
        .map_err(|e| {
            let msg = format!("pipeline_status size-walk join error: {e}");
            log::warn!("[memory-tree][rpc] pipeline_status: {msg}");
            msg
        })?;

    let is_paused = config.scheduler_gate.mode == SchedulerGateMode::Off;
    let is_syncing = pipeline_jobs.running > 0;

    // #002: read the degradation snapshot (set by the embed / extract stages)
    // so a half-working sync surfaces as `degraded` with a cause rather than a
    // misleading `running`. The structure-degraded latch is a liveness signal
    // ("the extraction model is timing out") kept honest at its source in the
    // driver's `extract::llm` — it self-clears on the next *completed*
    // extraction (#3365), so the status surface never consults the unrelated
    // `extraction_coverage` metric to second-guess it here.
    //
    // Asked of the driver rather than of this process's statics (#5560). Those
    // flags live inside whichever process ran the embed and extract stages, and
    // that is the module — a `cdylib` with its own statics — so the host-side
    // read answered all-clear no matter what the pipeline had done. Same class
    // of bug, and same fix, as `backfill_in_progress` above.
    //
    // Degraded to all-clear on a read failure, matching the two other reads
    // this handler must not fail on (`backfill_in_progress` and
    // `latest_failed_job_failure`): a status surface that errors tells the user
    // less than one reporting an undegraded store, and all-clear is what the
    // host-side read answered anyway.
    let degraded = crate::openhuman::memory::tree::health::report::current_degraded_state(config)
        .await
        .unwrap_or_else(|error| {
            log::warn!("[memory-tree][rpc] pipeline_status: degraded state read failed: {error}");
            Default::default()
        });

    let (status, reason) = derive_pipeline_status(
        is_paused,
        config.scheduler_gate.mode,
        is_syncing,
        pipeline_jobs.failed,
        failed_unrecoverable,
        total_chunks,
        &degraded,
        queue_idle_ms,
    );

    // #002 first_blocking_cause (FR-004): the most-recent failed job's typed
    // reason, surfaced verbatim by the UI. Best-effort — log-then-drop, so a
    // read failure is distinguishable in the log from "no blocking cause"
    // while never failing the polled status RPC. The `spawn_blocking` this
    // used to sit in is the driver's business now.
    let latest_failure = latest_failed_job_failure(config).await.unwrap_or_else(|e| {
        log::warn!(
            "[memory-tree][rpc] pipeline_status: latest_failed_job_failure read failed: {e}"
        );
        None
    });

    // #002 extraction_coverage (FR-010/US5): fraction of chunks with
    // structure, surfaced as its own display metric — deliberately NOT folded
    // into the status pill (#3365: coverage is a cumulative measure, unrelated
    // to the live structure-degraded liveness signal).
    //
    // Derived from the `store_stats` snapshot above, so the numerator and
    // denominator are one observation and the fraction cannot exceed 1.0 —
    // which two separate reads could produce, and did.
    //
    // An empty store still reports `Some(0.0)`, matching what this returned
    // before. `None` here has always meant "unavailable", and while `0.0` for
    // a store with nothing to extract is arguably the wrong reading, changing
    // it is a decision about what the panel shows, not a consequence of moving
    // the read behind the contract.
    let extraction_coverage = Some(if store.chunks == 0 {
        0.0
    } else {
        store.chunks_with_structure as f32 / store.chunks as f32
    });

    // A hard failed-job reason is more urgent than a soft degradation; fall
    // back to the active degradation cause, then `None` when healthy.
    let first_blocking_cause = latest_failure.or_else(|| degraded.cause.clone());

    // openhuman#5820: disk-derived so it is durable and replayable; "resynced"
    // reads the same `store` observation as the chunk tile, so the two agree.
    let quarantine = latest_quarantine(config.workspace_dir.as_path(), total_chunks);
    if let Some(q) = &quarantine {
        log::debug!(
            "[memory-tree][rpc] pipeline_status: quarantine at={} resynced={} path={}",
            q.quarantined_at_ms,
            q.resynced,
            q.quarantined_path
        );
    }

    let payload = PipelineStatusResponse {
        status: status.clone(),
        reason: reason.clone(),
        last_sync_ms,
        total_chunks,
        wiki_size_bytes,
        pipeline_jobs,
        is_syncing,
        is_paused,
        degraded,
        first_blocking_cause,
        extraction_coverage,
        quarantine,
    };

    log::debug!(
        "[memory-tree][rpc] pipeline_status: ok status={status} total_chunks={total_chunks} wiki_size_bytes={wiki_size_bytes} ready={r} running={n} failed={f} reason={reason:?}",
        r = payload.pipeline_jobs.ready,
        n = payload.pipeline_jobs.running,
        f = payload.pipeline_jobs.failed,
    );

    Ok(RpcOutcome::single_log(
        payload,
        format!(
            "memory_tree: pipeline_status status={status} total_chunks={total_chunks} is_paused={is_paused} is_syncing={is_syncing}",
        ),
    ))
}

/// `memory_tree_doctor` RPC handler (#002 FR-009). Runs the one-shot
/// pipeline diagnostic and returns the
/// [`DoctorReport`](crate::openhuman::memory::tree::health::report::DoctorReport)
/// — per-stage health, the first blocking cause, the degraded snapshot, and
/// counters. Exposed for the agent tool + CLI so the agent can self-diagnose an
/// empty/stalled wiki.
///
/// The pass runs inside the driver now (`MemoryMaintenance::diagnose`), which
/// is also where the blocking SQLite reads it makes have always been — this
/// host no longer dispatches a blocking task for them, and the report's shape
/// is unchanged.
pub async fn doctor_rpc(
    config: &Config,
) -> Result<RpcOutcome<crate::openhuman::memory::tree::health::report::DoctorReport>, String> {
    let report = crate::openhuman::memory::tree::health::report::run_doctor(config).await;
    let summary = if report.healthy {
        "memory_tree: doctor — healthy".to_string()
    } else {
        format!(
            "memory_tree: doctor — first_blocking_cause={}",
            report
                .first_blocking_cause
                .as_ref()
                .map(|f| f.code.as_str())
                .unwrap_or("unknown")
        )
    };
    Ok(RpcOutcome::single_log(report, summary))
}

/// Response from `memory_tree_retry_failed` (#002 FR-011).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryFailedResponse {
    /// Number of `failed` jobs flipped back to `ready` for retry.
    pub requeued: u64,
}

/// `memory_tree_retry_failed` RPC handler (#002 FR-011). Flips every
/// terminally-`failed` `mem_tree_jobs` row back to `ready` (fresh attempt
/// budget, typed reason cleared) so jobs that failed under a now-fixed config
/// re-run without re-ingesting source data. Backs the "Retry failed" button.
pub async fn retry_failed_rpc(config: &Config) -> Result<RpcOutcome<RetryFailedResponse>, String> {
    // Requeue and wake are one operation at the driver. They were two calls
    // here, which is one call away from a retry that moves rows and then lets
    // them sit until the next scheduled window.
    let requeued = crate::openhuman::memory::ops::maintenance::retry_failed(config).await?;
    Ok(RpcOutcome::single_log(
        RetryFailedResponse { requeued },
        format!("memory_tree: retry_failed requeued={requeued}"),
    ))
}

/// #002 (FR-004): the typed [`PipelineFailure`] of the most-recently-failed
/// `mem_tree_jobs` row, when it carries a classified `failure_reason` **and that
/// failure is still the pipeline's current blocking cause**. Returns `Ok(None)`
/// when there is no failed job with a typed reason (older failures predating the
/// typed-failure columns, or none at all), or when the failure has been
/// superseded (below). Best-effort: the status panel is a UI convenience, so a
/// DB error degrades to `Ok(None)` rather than failing the whole status RPC.
///
/// # Supersession — why the newest failed row is not automatically the cause
///
/// An unrecoverable failure is terminal by design: it is never retried, so its
/// row sits in `failed` forever with whatever `failure_reason` it died with.
/// Reading that row unconditionally means the panel keeps rendering the *first*
/// diagnosis it ever saw, indefinitely, no matter what the pipeline has done
/// since.
///
/// In production that surfaced as a signed-in user being told "No embeddings
/// credentials found. Log in to OpenHuman" — the remediation for an
/// `auth_missing` batch that had failed **27 days earlier**, while the queue had
/// been completing jobs normally the whole time. The banner was a tombstone, and
/// following it was impossible: the user was already logged in.
///
/// So a failure only counts as the *current* blocking cause when the queue has
/// not settled a job successfully since it. `completed_at_ms` on the newest
/// `done` row is that watermark: if the pipeline has produced output more
/// recently than the failure, the failure describes the past, not the present.
/// The failure is still counted (`failed_unrecoverable` keeps the status at
/// `error` and the "N unrecoverable failure(s) need action" reason), and "Retry
/// failed" is how the user clears it — but the *remediation text*, which tells
/// the user what to go and do right now, is withheld once it stops being true.
async fn latest_failed_job_failure(
    config: &Config,
) -> Result<Option<crate::openhuman::memory::tree::health::PipelineFailure>, String> {
    // The failure and the success watermark arrive together, as ONE answer.
    // That is not a convenience: asking twice lets a job settle between the
    // two and flip the supersession decision below, which is the race the
    // #5427 review flagged. The driver reads both on one connection; taking
    // `QueueFailure::last_success_ms` from a second call would undo that.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory-tree][rpc] pipeline_status: driver '{}' does not serve Maintenance; no blocking cause",
            binding.driver_id()
        );
        return Ok(None);
    };
    let reported = maintenance
        .latest_queue_failure()
        .await
        .map_err(|e| format!("latest_failed_job_failure: {e}"))?;
    Ok(reported.as_ref().and_then(blocking_cause))
}

/// The supersession rule, over one failure the driver reported.
///
/// Split from the fetch above so it stays exercisable without a bound driver.
/// The rule is the part with edge cases — an untimestamped failure, a
/// watermark on the same millisecond, a reason this build does not know — and
/// a test that has to stand up a driver to reach it tends not to cover them.
fn blocking_cause(
    reported: &crate::openhuman::memory::api::provider::types::QueueFailure,
) -> Option<crate::openhuman::memory::tree::health::PipelineFailure> {
    use crate::openhuman::memory::tree::health::{FailureClass, FailureCode, PipelineFailure};

    let reason = &reported.reason;
    let class = reported.class.clone();
    let failed_at_ms = reported.completed_at_ms;
    let last_success_ms = reported.last_success_ms;

    // Log every supersession branch, not only the withheld one, so the decision
    // is greppable from the logs alone.
    match failed_at_ms {
        Some(failed_at_ms)
            if last_success_ms.is_some_and(|success_ms| success_ms > failed_at_ms) =>
        {
            log::debug!(
                "[memory-tree][rpc] pipeline_status: withholding blocking cause reason={reason} \
                 — the queue has completed a job since it failed (superseded)"
            );
            return None;
        }
        Some(_) => {
            log::debug!(
                "[memory-tree][rpc] pipeline_status: blocking cause is live reason={reason} \
                 — no successful settle since it failed"
            );
        }
        None => {
            log::debug!(
                "[memory-tree][rpc] pipeline_status: blocking cause reason={reason} has no \
                 completion timestamp — surfacing unconditionally (legacy row)"
            );
        }
    }

    // A reason this build has no code for is not a cause it can render.
    let code = FailureCode::from_str(reason)?;
    // Trust the persisted class when present and parseable; otherwise derive
    // from the code (keeps a forward-compatible default if the column is NULL
    // on an older row).
    let mut failure = PipelineFailure::new(code);
    if let Some(c) = class.as_deref() {
        if c == "transient" {
            failure.class = FailureClass::Transient;
        } else if c == "unrecoverable" {
            failure.class = FailureClass::Unrecoverable;
        }
    }
    Some(failure)
}

/// #5324: how long the queue has been sitting on eligible work without
/// finishing anything, or `None` when there is no eligible work waiting.
///
/// This is the "queued but never processed" signal. Getting the predicate
/// right matters more than it looks, because the naive versions produce false
/// alarms for exactly the heavy users this issue is about:
///
/// - **Not** `MIN(created_at_ms)` over all `ready` rows. `mark_deferred` parks
///   a backing-off job by leaving `status = 'ready'` and pushing
///   `available_at_ms` forward, so deferred work would count as waiting when
///   it is deliberately asleep.
/// - **Not** the age of the oldest eligible row either. A re-embed backfill
///   enqueues thousands of rows in one burst; six hours into a perfectly
///   healthy drain of a 68k-chunk workspace, the oldest un-drained row is by
///   definition hours old. That would flag the exact case the issue's reporter
///   was in — a big, slow, *working* backfill — as broken.
///
/// So the measure is **idle time, not backlog age**: how long since the queue
/// last settled *any* job. A pipeline making progress refreshes
/// `completed_at_ms` continuously no matter how deep the backlog is, while a
/// pipeline whose jobs all fail unrecoverably or whose worker never runs goes
/// quiet. `completed_at_ms` is stamped on failure as well as success, so a
/// fast-failing pipeline reports `error` (via `failed_unrecoverable`) rather
/// than being mislabelled as stalled.
///
/// Returns `Some(idle_ms)` only when eligible work is actually waiting — an
/// idle queue with nothing to do is not stalled, it is done. When nothing has
/// ever settled (fresh workspace whose worker has never run), idle time falls
/// back to how long the oldest eligible job has been waiting.
///
/// Derived from a snapshot rather than read on its own, so the eligible count
/// and the timestamps it is measured against come from one instant. Reading
/// them separately is what makes an idle window appear across a settle that
/// happened between two queries.
fn queue_idle_ms(
    queue: &crate::openhuman::memory::api::provider::types::QueueStats,
    now_ms: i64,
) -> Option<i64> {
    // Nothing eligible is waiting ⇒ nothing is being held up.
    if queue.eligible_now == 0 {
        return None;
    }
    let (last_settled_ms, oldest_eligible_ms) = (queue.last_completed_ms, queue.oldest_eligible_ms);
    // Idle time is "how long since the queue last made progress on the work
    // that is waiting *now*" — so start the clock at the LATER of the last
    // settle and the oldest eligible job's arrival. Using `last_settled_ms`
    // alone (`.or`) mis-reads a real shape: if the queue drained everything,
    // sat empty for days, then a fresh job arrives, the stale completion is
    // hours/days old while the new work is seconds old. Taking the max means
    // freshly-enqueued work starts its own idle window instead of inheriting
    // an ancient completion, so a just-arrived job can't be flagged `degraded`
    // before the worker has had a chance to touch it. Fall back to the oldest
    // eligible job's wait when the queue has never settled a job at all.
    let reference_ms = match (last_settled_ms, oldest_eligible_ms) {
        (Some(last_settled), Some(oldest_eligible)) => Some(last_settled.max(oldest_eligible)),
        (Some(last_settled), None) => Some(last_settled),
        (None, Some(oldest_eligible)) => Some(oldest_eligible),
        (None, None) => None,
    };
    // Clamp at zero: clock skew / a future-dated row must read as "just now",
    // never as a negative age.
    reference_ms.map(|since| (now_ms - since).max(0))
}
