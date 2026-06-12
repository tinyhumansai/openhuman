//! One-shot memory-pipeline diagnostic (#002 FR-009).
//!
//! `run_doctor` walks each stage of the chunk→wiki + summary-tree pipeline and
//! returns a [`DoctorReport`]: per-stage health, the single first blocking
//! cause (so the agent / CLI gets one actionable answer instead of a wall of
//! counters), and the current counters. It is exposed as an agent tool and a
//! CLI/RPC method — there is no UI surface this round (the status panel
//! already renders `first_blocking_cause`).
//!
//! Design: this is a **config + persisted-state** diagnosis — it reads the
//! routing config, the scheduler-gate mode, the process-global degraded flags
//! (set by the embed/extract stages), the job-queue counters, and the chunk
//! count. It intentionally does **not** fire a live embed/extract probe in this
//! cut: a network call would make the doctor slow, flaky, and order-dependent,
//! and the degraded flags already capture "did the last real run fail and how".
//! A time-boxed live probe is a clean follow-up if we want pre-run validation.

use serde::{Deserialize, Serialize};

use super::{current_degraded_state, DegradedState, FailureCode, PipelineFailure};
use crate::openhuman::config::{Config, SchedulerGateMode};

/// Health of one named pipeline stage.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageHealth {
    /// Stable stage id: `routing`, `scheduler_gate`, `embeddings`,
    /// `extraction`, `queue`, `summary_tree`.
    pub stage: String,
    /// True when this stage is healthy / not blocking.
    pub ok: bool,
    /// Typed failure when `ok == false`; `None` when healthy. Carries the
    /// i18n remediation key the surfaces render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<PipelineFailure>,
    /// Short non-localized human note for logs / CLI (never a secret).
    pub note: String,
}

impl StageHealth {
    fn ok(stage: &str, note: impl Into<String>) -> Self {
        Self {
            stage: stage.to_string(),
            ok: true,
            failure: None,
            note: note.into(),
        }
    }

    fn bad(stage: &str, failure: PipelineFailure, note: impl Into<String>) -> Self {
        Self {
            stage: stage.to_string(),
            ok: false,
            failure: Some(failure),
            note: note.into(),
        }
    }
}

/// Current pipeline counters, mirrored from the status surface so the doctor
/// is a one-call snapshot.
// No `Eq`: `extraction_coverage` is `Option<f32>` — `f32` never implements `Eq`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DoctorCounters {
    pub total_chunks: u64,
    pub jobs_ready: u64,
    pub jobs_running: u64,
    pub jobs_failed: u64,
    /// #002 (FR-010 / US5): fraction of chunks with ≥1 indexed entity, in
    /// `[0.0, 1.0]`. Near 0 with `total_chunks > 0` means extraction is
    /// producing no structure. `None` when the metric could not be measured
    /// (DB read error) — deliberately distinct from a genuine `0.0` so a
    /// broken measurement is never misreported as a structure failure.
    #[serde(default)]
    pub extraction_coverage: Option<f32>,
}

/// The full diagnostic. `first_blocking_cause` is the failure of the first
/// non-ok stage in pipeline order (`stages` is already ordered), so a caller
/// can act on one thing; `healthy` is the convenience roll-up.
// No `Eq`: transitively contains `DoctorCounters` (Option<f32> — f32: !Eq).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DoctorReport {
    pub healthy: bool,
    pub stages: Vec<StageHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_blocking_cause: Option<PipelineFailure>,
    pub degraded: DegradedState,
    pub counters: DoctorCounters,
}

/// Run the diagnostic against `config` + persisted/queue/degraded state.
///
/// Best-effort: counter reads that error degrade to 0 (the doctor is a
/// convenience, not an audit) and never fail the whole call. Stage order is
/// the pipeline order so the first non-ok stage is the first blocking cause.
pub fn run_doctor(config: &Config) -> DoctorReport {
    use crate::openhuman::memory_queue::store as queue;
    use crate::openhuman::memory_queue::types::JobStatus;
    use crate::openhuman::memory_store::chunks::store as chunks;

    let degraded = current_degraded_state();
    let counters = DoctorCounters {
        total_chunks: chunks::count_chunks(config).unwrap_or(0),
        jobs_ready: queue::count_by_status(config, JobStatus::Ready).unwrap_or(0),
        jobs_running: queue::count_by_status(config, JobStatus::Running).unwrap_or(0),
        jobs_failed: queue::count_by_status(config, JobStatus::Failed).unwrap_or(0),
        extraction_coverage: chunks::extraction_coverage(config).ok(),
    };

    let mut stages = Vec::new();

    // 1. Routing/config sanity — is *any* embeddings provider configured?
    //    (`build_write_embedder` skips embedding when none is, so this is the
    //    most common "empty wiki" root cause.)
    let embeddings_provider = config
        .memory_tree
        .embedding_endpoint
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|_| "ollama-override".to_string())
        .or_else(|| config.embeddings_provider.clone())
        .filter(|s| !s.trim().is_empty());
    stages.push(match embeddings_provider.as_deref() {
        // Explicit `none` opt-out: semantic recall is off by the user's choice,
        // not a fault. Reported `ok` (consistent with a `scheduler_gate=off`
        // pause and the write-path opt-out treatment) but with an honest note,
        // so the prior "provider configured: none" can't read as a working
        // embeddings provider. (CodeRabbit on doctor.rs)
        Some("none") => StageHealth::ok(
            "embeddings",
            "embeddings disabled by you (provider = none) — semantic recall is intentionally off",
        ),
        Some(p) => StageHealth::ok("embeddings", format!("provider configured: {p}")),
        None => StageHealth::bad(
            "embeddings",
            PipelineFailure::new(FailureCode::EmbeddingsUnconfigured),
            "no embeddings provider configured — semantic recall is off",
        ),
    });

    // 2. Scheduler gate — `off` means the user paused background work. Report
    //    it as a *user choice*, not a fault (ok == true), but note it so a
    //    confused "nothing is happening" reads clearly.
    let gate_off = config.scheduler_gate.mode == SchedulerGateMode::Off;
    stages.push(StageHealth::ok(
        "scheduler_gate",
        if gate_off {
            "paused by you (scheduler gate = off) — background sync is intentionally stopped"
        } else {
            "auto — background sync runs"
        },
    ));

    // 3. Queue health — failed jobs are a hard signal. The typed reason (when
    //    present on the most-recent failed row) is surfaced by the status RPC;
    //    here we just flag that failures exist and how many.
    if counters.jobs_failed > 0 {
        stages.push(StageHealth::bad(
            "queue",
            // The most-recent typed reason is surfaced by pipeline_status;
            // doctor reports the count + a transient-by-default placeholder so
            // the stage is non-ok and actionable.
            PipelineFailure::new(FailureCode::Transient),
            format!("{} failed job(s) in mem_tree_jobs", counters.jobs_failed),
        ));
    } else {
        stages.push(StageHealth::ok("queue", "no failed jobs"));
    }

    // 4. Degraded signals from the last real run.
    if degraded.semantic_recall {
        let cause = degraded
            .cause
            .clone()
            .unwrap_or_else(|| PipelineFailure::new(FailureCode::EmbeddingsUnconfigured));
        stages.push(StageHealth::bad(
            "extraction",
            // semantic_recall degradation is an embeddings problem, but reuse
            // the recorded cause which names the real reason.
            cause,
            "semantic recall degraded — embeddings were skipped on the last run",
        ));
    } else if degraded.structure {
        let cause = degraded
            .cause
            .clone()
            .unwrap_or_else(|| PipelineFailure::new(FailureCode::ExtractionTimeout));
        stages.push(StageHealth::bad(
            "extraction",
            cause,
            "wiki structure degraded — extraction produced no entities on the last run",
        ));
    } else {
        stages.push(StageHealth::ok("extraction", "no degradation recorded"));
    }

    // 5. Summary-tree precondition. Reuse the runtime's own capability check
    //    (`tree_runtime::ops::summarizer_available`) so the doctor matches what
    //    "Build Summary Trees" will actually do — since #002 FR-007 it runs on
    //    the configured cloud provider when local AI is off, so local-AI-off is
    //    NOT a fault by itself. Only `bad` when no provider resolves at all.
    let (summary_ok, summary_note) =
        crate::openhuman::memory_tree::tree_runtime::ops::summarizer_available(config);
    stages.push(if summary_ok {
        StageHealth::ok("summary_tree", summary_note)
    } else {
        StageHealth::bad(
            "summary_tree",
            PipelineFailure::new(FailureCode::SummarizerUnavailable),
            summary_note,
        )
    });

    let first_blocking_cause = stages
        .iter()
        .find(|s| !s.ok)
        .and_then(|s| s.failure.clone());
    let healthy = first_blocking_cause.is_none();

    DoctorReport {
        healthy,
        stages,
        first_blocking_cause,
        degraded,
        counters,
    }
}

/// Async wrapper around [`run_doctor`] for async call sites (the RPC + agent
/// tool). `run_doctor` does synchronous SQLite reads (chunk/job counts +
/// extraction coverage); a contended DB could pin a Tokio worker for the
/// busy-timeout window, so offload the whole diagnostic to a blocking thread.
pub async fn async_run_doctor(config: &Config) -> DoctorReport {
    let cfg = config.clone();
    match tokio::task::spawn_blocking(move || run_doctor(&cfg)).await {
        Ok(report) => report,
        Err(join_err) => {
            // The blocking task panicked — surface a degraded-but-shaped report
            // rather than propagating, since the doctor is a best-effort
            // diagnostic and callers expect a report, not an error.
            log::warn!("[memory_tree::health::doctor] run_doctor task failed: {join_err}");
            DoctorReport {
                healthy: false,
                stages: Vec::new(),
                first_blocking_cause: Some(PipelineFailure::new(FailureCode::Transient)),
                degraded: current_degraded_state(),
                counters: DoctorCounters::default(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        (tmp, cfg)
    }

    #[test]
    fn misconfigured_workspace_reports_embeddings_as_first_blocking_cause() {
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = None; // no provider at all
        cfg.local_ai.runtime_enabled = false;

        let report = run_doctor(&cfg);
        assert!(!report.healthy);
        // Embeddings is stage 1, so it is the first blocking cause.
        let cause = report.first_blocking_cause.expect("should have a cause");
        assert_eq!(cause.code, FailureCode::EmbeddingsUnconfigured);
        // The embeddings stage is non-ok with the same code.
        let embed = report
            .stages
            .iter()
            .find(|s| s.stage == "embeddings")
            .unwrap();
        assert!(!embed.ok);
    }

    #[test]
    fn healthy_when_embeddings_and_local_ai_configured() {
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("none".into()); // a configured choice
        cfg.local_ai.runtime_enabled = true;

        let report = run_doctor(&cfg);
        assert!(
            report.healthy,
            "expected healthy, got {:?}",
            report.first_blocking_cause
        );
        assert!(report.first_blocking_cause.is_none());
        // Every stage ok.
        assert!(
            report.stages.iter().all(|s| s.ok),
            "stages: {:?}",
            report.stages
        );
    }

    #[test]
    fn embeddings_none_opt_out_is_ok_but_note_is_honest() {
        // `embeddings_provider = "none"` is a deliberate opt-out: the stage stays
        // ok (a configured choice, like a paused scheduler gate) but the note must
        // not read as a working provider ("provider configured: none"). (CodeRabbit)
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("none".into());
        cfg.local_ai.runtime_enabled = true;

        let report = run_doctor(&cfg);
        let embed = report
            .stages
            .iter()
            .find(|s| s.stage == "embeddings")
            .unwrap();
        assert!(embed.ok, "opt-out is a choice, not a fault");
        assert!(
            embed.note.contains("disabled") && embed.note.contains("intentionally off"),
            "note must name the intentional opt-out, got: {}",
            embed.note
        );
        assert!(
            !embed.note.contains("provider configured"),
            "must not read as a working provider, got: {}",
            embed.note
        );
    }

    #[test]
    fn scheduler_gate_off_is_a_choice_not_a_fault() {
        use crate::openhuman::config::SchedulerGateMode;
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("ollama:bge-m3".into());
        cfg.local_ai.runtime_enabled = true;
        cfg.scheduler_gate.mode = SchedulerGateMode::Off;

        // Double-reset: guard resets on entry, but a concurrent non-guarded
        // code path (e.g. a tokio task draining after its test dropped its
        // guard) may have re-set the flags between guard acquisition and here.
        super::super::clear_semantic_recall_degraded();
        super::super::clear_structure_degraded();

        let report = run_doctor(&cfg);
        // Paused is reported but does NOT make the pipeline unhealthy.
        assert!(
            report.healthy,
            "expected healthy, failing stages: {:?}",
            report.stages.iter().filter(|s| !s.ok).collect::<Vec<_>>()
        );
        let gate = report
            .stages
            .iter()
            .find(|s| s.stage == "scheduler_gate")
            .unwrap();
        assert!(gate.ok);
        assert!(gate.note.contains("paused"));
    }

    /// #002 FR-007 / Gray review: the doctor's `summary_tree` stage must mirror
    /// `summarizer_available` exactly. With local AI off and no cloud opt-in
    /// (the default), the stage reports unavailable — which is correct, since
    /// cloud summarization requires explicit consent. The stage must NOT fire
    /// a generic "local AI required" hard-failure; it names the opt-in gap.
    #[test]
    fn local_ai_off_reports_no_provider_without_cloud_opt_in() {
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("ollama:bge-m3".into()); // embeddings ok
        cfg.local_ai.runtime_enabled = false; // cloud opt-in not set (default false)

        let report = run_doctor(&cfg);
        let tree = report
            .stages
            .iter()
            .find(|s| s.stage == "summary_tree")
            .unwrap();
        // summary_tree must mirror summarizer_available precisely.
        assert_eq!(
            tree.ok,
            crate::openhuman::memory_tree::tree_runtime::ops::summarizer_available(&cfg).0,
            "summary_tree health must mirror the runtime capability check"
        );
        // Without opt-in, the note names the "no summarization provider" case.
        assert!(
            tree.note.contains("no summarization provider"),
            "unexpected summary_tree note: {}",
            tree.note
        );
    }

    #[test]
    fn report_serde_roundtrips() {
        let _g = super::super::test_guard();
        let (_tmp, cfg) = test_config();
        let report = run_doctor(&cfg);
        let json = serde_json::to_string(&report).unwrap();
        let back: DoctorReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }
}
