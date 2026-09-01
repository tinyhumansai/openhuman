use super::*;

#[test]
fn derive_pipeline_status_precedence_matches_spec() {
    use crate::openhuman::memory::tree::health::{DegradedState, FailureCode, PipelineFailure};
    use tinymemory_api::host::SchedulerGateMode;

    let healthy = DegradedState::default();
    let recall_degraded = DegradedState {
        semantic_recall: true,
        structure: false,
        storage: false,
        cause: Some(PipelineFailure::new(FailureCode::EmbeddingsUnconfigured)),
    };
    let structure_degraded = DegradedState {
        semantic_recall: false,
        structure: true,
        storage: false,
        cause: Some(PipelineFailure::new(FailureCode::ExtractionTimeout)),
    };
    let storage_degraded = DegradedState {
        semantic_recall: false,
        structure: false,
        storage: true,
        cause: Some(PipelineFailure::new(FailureCode::StorageUnavailable)),
    };

    // Args: (is_paused, mode, is_syncing, failed, failed_unrecoverable,
    //        total_chunks, &degraded, queue_idle_ms).

    // paused beats everything else (even degradation)
    let (s, reason) = derive_pipeline_status(
        true,
        SchedulerGateMode::Off,
        true,
        5,
        5,
        100,
        &recall_degraded,
        None,
    );
    assert_eq!(s, "paused");
    assert!(reason.unwrap().contains("off"));

    // paused still beats a storage failure (user explicitly stood the
    // worker down; the flag won't be freshly set anyway).
    let (s, _) = derive_pipeline_status(
        true,
        SchedulerGateMode::Off,
        false,
        0,
        0,
        0,
        &storage_degraded,
        None,
    );
    assert_eq!(s, "paused", "paused beats storage");

    // storage failure → error, and it fires even with ZERO chunks (unlike
    // recall/structure degradation, which is content-relative) — a dead
    // disk is broken regardless of how much content exists.
    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        0, // no chunks — must still surface
        &storage_degraded,
        None,
    );
    assert_eq!(
        s, "error",
        "storage failure is a hard error at any chunk count"
    );
    assert!(reason.unwrap().contains("storage"));

    // storage outranks transient-failed degradation too.
    let (s, _) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        true,
        3,
        0,
        100,
        &storage_degraded,
        None,
    );
    assert_eq!(s, "error", "storage beats transient-degraded");

    // error beats degraded / syncing / running / idle — but ONLY for
    // unrecoverable failures (#3365).
    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        true,
        2,
        2, // both failures unrecoverable
        100,
        &recall_degraded,
        None,
    );
    assert_eq!(s, "error");
    assert!(reason.unwrap().contains("unrecoverable"));

    // #3365: transient-only failures (failed > 0, none unrecoverable) do NOT
    // escalate to error — they self-heal via auto-requeue, so they surface
    // as `degraded` ("retrying"), beating syncing/running.
    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        true,
        3,
        0,
        100,
        &healthy,
        None,
    );
    assert_eq!(s, "degraded", "transient failures must not read as error");
    assert!(reason.unwrap().contains("3 job(s) failed, retrying"));

    // #002: degraded beats syncing / running / idle (but loses to paused/error)
    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        true, // syncing
        0,
        0,
        100,
        &recall_degraded,
        None,
    );
    assert_eq!(s, "degraded", "degraded must beat syncing");
    assert!(reason.unwrap().contains("semantic recall disabled"));

    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        100,
        &structure_degraded,
        None,
    );
    assert_eq!(s, "degraded");
    assert!(reason.unwrap().contains("wiki structure incomplete"));

    // syncing beats running / idle (when healthy)
    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        true,
        0,
        0,
        100,
        &healthy,
        None,
    );
    assert_eq!(s, "syncing");
    assert!(reason.is_none());

    // running when chunks exist but nothing in flight
    let (s, _) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        100,
        &healthy,
        None,
    );
    assert_eq!(s, "running");

    // idle when the store is empty and nothing is in flight (transient
    // failures with no content don't manufacture a `degraded`).
    let (s, _) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        2,
        0,
        0,
        &healthy,
        None,
    );
    assert_eq!(s, "idle");
}

/// #5324: a queue that accepts work but never drains it must report
/// `degraded`, not the `running`/`idle` that made a month-long outage look
/// healthy. Pins the threshold boundary and the full precedence chain.
#[test]
fn stalled_queue_degrades_instead_of_reading_healthy() {
    use crate::openhuman::memory::tree::health::{DegradedState, FailureCode, PipelineFailure};
    use tinymemory_api::host::SchedulerGateMode;

    let healthy = DegradedState::default();
    let stalled = Some(QUEUE_STALL_THRESHOLD_MS);
    let just_under = Some(QUEUE_STALL_THRESHOLD_MS - 1);

    // The regression itself: chunks exist, nothing failed, nothing running
    // — previously "running", which is what let the outage hide.
    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        100,
        &healthy,
        stalled,
    );
    assert_eq!(s, "degraded", "a stalled queue must not read as running");
    assert!(reason.unwrap().contains("has not completed any job"));

    // NOT gated on total_chunks: a queue that never drained has no chunks,
    // and that case must not read as `idle`.
    let (s, _) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        0,
        &healthy,
        stalled,
    );
    assert_eq!(s, "degraded", "empty-but-stalled must not read as idle");

    // Boundary: one millisecond under the threshold is still healthy, so a
    // merely slow flush window can't trip it.
    let (s, _) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        100,
        &healthy,
        just_under,
    );
    assert_eq!(s, "running", "under the threshold stays healthy");

    // `None` (no ready jobs, or an unreadable metric) never manufactures a
    // degraded verdict.
    let (s, _) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        0,
        0,
        100,
        &healthy,
        None,
    );
    assert_eq!(s, "running", "absent metric must not claim a stall");

    // Precedence: paused and error both outrank the stall — a typed
    // unrecoverable failure is the more specific, more actionable answer.
    let (s, _) = derive_pipeline_status(
        true,
        SchedulerGateMode::Off,
        false,
        0,
        0,
        100,
        &healthy,
        stalled,
    );
    assert_eq!(s, "paused", "paused beats stalled");

    let (s, reason) = derive_pipeline_status(
        false,
        SchedulerGateMode::Auto,
        false,
        1,
        1,
        100,
        &healthy,
        stalled,
    );
    assert_eq!(s, "error", "unrecoverable failure beats stalled");
    assert!(reason.unwrap().contains("unrecoverable"));

    // Sanity: the budget-exhausted failure this issue is about is indeed
    // classified unrecoverable, so it lands in the `error` branch above and
    // carries its own remediation key.
    let budget = PipelineFailure::new(FailureCode::BudgetExhausted);
    assert!(budget.is_unrecoverable());
    assert_eq!(
        budget.remediation_key,
        "memory.health.remediation.budget_exhausted"
    );
}

#[tokio::test]
async fn queue_idle_ms_ignores_deep_but_draining_and_deferred_backlogs() {
    let now = 1_800_000_000_000_i64;
    let long_ago = now - 48 * 60 * 60 * 1000;

    // Nothing queued at all ⇒ not stalled (an empty queue is done, not stuck).
    assert_eq!(queue_idle_ms(&queue(0, None, None), now), None);

    // A deep backlog whose oldest eligible job arrived 48h ago, and which
    // has never settled anything. A naive backlog-age metric reads 48h and
    // cries "stalled" — and here it is right, because a queue that has
    // never settled a job IS stalled.
    assert!(
        queue_idle_ms(&queue(3, None, Some(long_ago)), now).unwrap() >= QUEUE_STALL_THRESHOLD_MS,
        "a queue that has never settled a job IS stalled"
    );

    // Same 48h-old backlog, but one job settled a minute ago — the
    // pipeline is draining. This is the shape a backlog-age metric gets
    // wrong, and it describes the heavy users this issue is about.
    let idle = queue_idle_ms(&queue(3, Some(now - 60_000), Some(long_ago)), now)
        .expect("work still queued");
    assert!(
        idle < QUEUE_STALL_THRESHOLD_MS,
        "a deep but draining backlog must not read as stalled (idle={idle}ms)"
    );

    // Wholly deferred: every job is backing off, so nothing is runnable.
    // Asleep on purpose is not stuck.
    assert_eq!(
        queue_idle_ms(&queue(0, Some(long_ago), None), now),
        None,
        "wholly-deferred work is asleep, not stalled"
    );
}

/// #5324 regression (CodeRabbit/Codex): a queue that drained everything,
/// sat quiet for two days, then received one fresh eligible job must start
/// the idle clock at the NEW job's arrival — not inherit the ancient
/// completion. The prior `last_settled_ms.or(oldest_eligible_ms)` picked
/// the stale 48h-old settle and reported `degraded` the instant new work
/// appeared, before the worker had any chance to touch it.
#[tokio::test]
async fn queue_idle_ms_starts_from_fresh_work_not_ancient_completion() {
    let now = 1_800_000_000_000_i64;
    let long_ago = now - 48 * 60 * 60 * 1000;
    let just_now = now - 60_000;

    // The queue settled its last job 48h ago and went quiet; one fresh
    // eligible job arrived a minute ago.
    let idle = queue_idle_ms(&queue(1, Some(long_ago), Some(just_now)), now)
        .expect("fresh work is waiting");
    assert!(
        idle < QUEUE_STALL_THRESHOLD_MS,
        "freshly-enqueued work must start its own idle window, not inherit a 48h-old \
         completion (idle={idle}ms)"
    );
    assert_eq!(
        idle,
        now - just_now,
        "the idle clock starts at the new job's arrival, not the stale settle"
    );
}

/// The active production defect: a signed-in user was told "No embeddings
/// credentials found. Log in to OpenHuman" because a batch of `auth_missing`
/// jobs had failed 27 days earlier and, being unrecoverable, was never
/// retried. The queue had been completing jobs the whole time since.
///
/// A failure the pipeline has already worked past is not the current
/// blocking cause, so no remediation is surfaced for it.
#[test]
fn blocking_cause_is_withheld_once_the_queue_has_succeeded_since() {
    let failed_at = 1_800_000_000_000_i64;
    let succeeded_after = failed_at + 27 * 24 * 60 * 60 * 1000;

    assert!(
        blocking_cause(&reported_failure(
            "auth_missing",
            Some(failed_at),
            Some(succeeded_after)
        ))
        .is_none(),
        "a month-old auth failure the queue has since worked past must not be \
         presented as the user's current problem"
    );
}

/// The other half of the same rule: a failure with no successful settle
/// after it IS the current blocking cause and must still surface, otherwise
/// the fix would silence the diagnosis it exists to deliver.
#[test]
fn blocking_cause_surfaces_when_nothing_has_succeeded_since() {
    use crate::openhuman::memory::tree::health::{FailureClass, FailureCode};

    let succeeded_before = 1_800_000_000_000_i64;
    let failed_after = succeeded_before + 60_000;

    let failure = blocking_cause(&reported_failure(
        "budget_exhausted",
        Some(failed_after),
        Some(succeeded_before),
    ))
    .expect("a failure with no success after it is the live cause");
    assert_eq!(failure.code, FailureCode::BudgetExhausted);
    assert_eq!(failure.class, FailureClass::Unrecoverable);
    assert_eq!(
        failure.remediation_key,
        "memory.health.remediation.budget_exhausted"
    );
}

/// A queue that has never completed anything has no watermark to compare
/// against, so the failure stands — this is the "broken from the first
/// sync" shape, where the diagnosis matters most.
#[test]
fn blocking_cause_surfaces_when_the_queue_has_never_succeeded() {
    let failure = blocking_cause(&reported_failure(
        "auth_invalid",
        Some(1_800_000_000_000_i64),
        None,
    ))
    .expect("no successful settle exists to supersede this failure");
    assert_eq!(
        failure.remediation_key,
        "memory.health.remediation.auth_invalid"
    );
}

/// A settle on the same millisecond as the failure does NOT supersede it.
///
/// The comparison is strictly `>`, and it has to be: `completed_at_ms` is
/// stamped on failure as well as success, so a job that fails and a job
/// that succeeds within the same millisecond are ordered by nothing. `>=`
/// would resolve that tie by hiding the failure, which is the direction
/// that loses a real diagnosis.
#[test]
fn a_settle_in_the_same_millisecond_does_not_supersede_the_failure() {
    let at = 1_800_000_000_000_i64;
    assert!(
        blocking_cause(&reported_failure("auth_invalid", Some(at), Some(at))).is_some(),
        "a success that cannot be shown to be later must not withhold the diagnosis"
    );
}

/// A failure carrying no completion time has nothing to compare against,
/// so it surfaces unconditionally rather than being withheld by a
/// watermark it cannot be ordered against.
#[test]
fn an_untimestamped_failure_surfaces_regardless_of_the_watermark() {
    let succeeded_at = 1_800_000_000_000_i64;
    assert!(
        blocking_cause(&reported_failure("auth_invalid", None, Some(succeeded_at))).is_some(),
        "without a failure timestamp there is no ordering, and withholding would \
         hide a live cause on a guess"
    );
}

/// On a fresh workspace the panel must report `idle` with zero
/// counters — the UI uses this to swap the loading skeleton for a
/// "no memory yet" state.
#[tokio::test]
async fn pipeline_status_returns_idle_for_empty_store() {
    // #002: the degraded flags are process-global; reset+serialise so a
    // parallel test (factory None-path, extract transport-fail) can't leak
    // a "degraded" signal into this fresh-workspace assertion.
    let _g = tinymemory_core::tree::health::test_guard();
    let (_tmp, cfg) = test_config();
    // An empty driver, bound explicitly. Without a binding installed this
    // resolves the real one, which means loading the compiled module — and
    // in a test process that blocks rather than failing.
    bind_diagnostics(&cfg, Default::default(), Default::default());
    let out = pipeline_status_rpc(&cfg).await.unwrap().value;
    assert_eq!(out.status, "idle");
    assert_eq!(out.total_chunks, 0);
    assert_eq!(out.last_sync_ms, 0);
    assert_eq!(out.pipeline_jobs.ready, 0);
    assert_eq!(out.pipeline_jobs.running, 0);
    assert_eq!(out.pipeline_jobs.failed, 0);
    assert!(!out.is_syncing);
    assert!(!out.is_paused);
    assert_eq!(out.wiki_size_bytes, 0, "no content dir yet");
    assert!(out.reason.is_none());
}

/// When the scheduler gate is `off`, the aggregated status flips to
/// `paused` regardless of the rest of the signals. This is the
/// invariant the toggle relies on.
#[tokio::test]
async fn pipeline_status_reflects_paused_when_scheduler_off() {
    use tinymemory_api::host::SchedulerGateMode;

    let (_tmp, mut cfg) = test_config();
    cfg.scheduler_gate.mode = SchedulerGateMode::Off;
    bind_diagnostics(&cfg, Default::default(), Default::default());
    let out = pipeline_status_rpc(&cfg).await.unwrap().value;
    assert_eq!(out.status, "paused");
    assert!(out.is_paused);
    let reason = out.reason.expect("paused must carry a reason");
    assert!(reason.contains("off"), "reason should name the mode");
}

/// `pipeline_status` renders the aggregates the driver reports, and
/// derives a terminal status from them.
///
/// This used to ingest a document and assert the counters moved. That
/// half — an ingest raising the chunk count — is the driver's, and is
/// pinned in the driver's conformance suite against a real store. What is
/// the host's, and what this pins, is that the reported numbers reach the
/// wire unchanged and that a populated, idle store reads as terminal
/// rather than syncing.
#[tokio::test]
async fn pipeline_status_renders_the_drivers_chunk_aggregates() {
    use crate::openhuman::memory::api::provider::types::{QueueStats, StoreStats};

    // #002: reset+serialise the process-global degraded flags so this
    // "running" assertion isn't flipped to "degraded" by a parallel test.
    let _g = tinymemory_core::tree::health::test_guard();
    let (_tmp, cfg) = test_config();

    let ingested_at = 1_800_000_000_000_i64;
    bind_diagnostics(
        &cfg,
        StoreStats {
            chunks: 4,
            chunks_with_structure: 1,
            most_recent_chunk_ms: Some(ingested_at),
        },
        QueueStats::default(),
    );

    let out = pipeline_status_rpc(&cfg).await.unwrap().value;
    assert_eq!(out.total_chunks, 4, "the driver's count reaches the wire");
    assert_eq!(
        out.last_sync_ms, ingested_at,
        "and so does its newest chunk's timestamp"
    );
    assert_eq!(
        out.extraction_coverage,
        Some(0.25),
        "coverage is the pair the driver reported, divided once"
    );
    // Provider availability differs between local and CI harnesses, so a
    // populated store may read as fully running or as degraded because
    // semantic recall or wiki structure was skipped. Both are terminal,
    // non-syncing states and both preserve the aggregates above.
    match out.status.as_str() {
        "running" => assert!(out.reason.is_none()),
        "degraded" => {
            let reason = out.reason.as_deref().unwrap_or_default();
            assert!(
                reason.contains("semantic recall disabled")
                    || reason.contains("wiki structure incomplete"),
                "degraded status should explain recall or structure loss: {:?}",
                out.reason
            );
        }
        other => panic!("expected running or degraded for a populated store, got {other}"),
    }
    assert!(!out.is_syncing);
}

/// `set_enabled` flips the persisted scheduler-gate mode and reports
/// `changed=true`; calling it again with the same value is a no-op
/// reporting `changed=false`. Uses an isolated `config_path` under
/// the workspace tempdir so `config.save()` doesn't touch the
/// host's real ~/.openhuman directory.
#[tokio::test]
async fn set_enabled_toggles_scheduler_gate_mode() {
    use tinymemory_api::host::SchedulerGateMode;

    let (tmp, mut cfg) = test_config();
    // Pin config_path inside the tempdir so `save()` stays sandboxed.
    cfg.config_path = tmp.path().join("config.toml");

    assert_eq!(cfg.scheduler_gate.mode, SchedulerGateMode::Auto);

    let off = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: false })
        .await
        .unwrap()
        .value;
    assert!(!off.enabled);
    assert!(off.changed);
    assert_eq!(off.mode, "off");
    assert_eq!(cfg.scheduler_gate.mode, SchedulerGateMode::Off);

    // Calling with the same value must report no-op.
    let again = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: false })
        .await
        .unwrap()
        .value;
    assert!(!again.changed, "duplicate toggle must be a no-op");

    // Flip back.
    let on = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: true })
        .await
        .unwrap()
        .value;
    assert!(on.enabled);
    assert!(on.changed);
    assert_eq!(on.mode, "auto");
    assert_eq!(cfg.scheduler_gate.mode, SchedulerGateMode::Auto);
}
