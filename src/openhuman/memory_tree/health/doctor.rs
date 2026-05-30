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
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorCounters {
    pub total_chunks: u64,
    pub jobs_ready: u64,
    pub jobs_running: u64,
    pub jobs_failed: u64,
}

/// The full diagnostic. `first_blocking_cause` is the failure of the first
/// non-ok stage in pipeline order (`stages` is already ordered), so a caller
/// can act on one thing; `healthy` is the convenience roll-up.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    stages.push(match &embeddings_provider {
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

    // 5. Summary-tree precondition — `tree_runtime::create_provider` requires
    //    local AI today. Flag it as a precondition (not a hard error) so the
    //    user knows "Build Summary Trees" needs local AI on this setup.
    stages.push(if config.local_ai.runtime_enabled {
        StageHealth::ok("summary_tree", "local AI enabled — Build Summary Trees can run")
    } else {
        StageHealth::bad(
            "summary_tree",
            PipelineFailure::new(FailureCode::LocalModelUnavailable),
            "local AI is off — Build Summary Trees requires it (or enable the cloud summarizer)",
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
        let embed = report.stages.iter().find(|s| s.stage == "embeddings").unwrap();
        assert!(!embed.ok);
    }

    #[test]
    fn healthy_when_embeddings_and_local_ai_configured() {
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("none".into()); // a configured choice
        cfg.local_ai.runtime_enabled = true;

        let report = run_doctor(&cfg);
        assert!(report.healthy, "expected healthy, got {:?}", report.first_blocking_cause);
        assert!(report.first_blocking_cause.is_none());
        // Every stage ok.
        assert!(report.stages.iter().all(|s| s.ok), "stages: {:?}", report.stages);
    }

    #[test]
    fn scheduler_gate_off_is_a_choice_not_a_fault() {
        use crate::openhuman::config::SchedulerGateMode;
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("ollama:bge-m3".into());
        cfg.local_ai.runtime_enabled = true;
        cfg.scheduler_gate.mode = SchedulerGateMode::Off;

        let report = run_doctor(&cfg);
        // Paused is reported but does NOT make the pipeline unhealthy.
        assert!(report.healthy);
        let gate = report.stages.iter().find(|s| s.stage == "scheduler_gate").unwrap();
        assert!(gate.ok);
        assert!(gate.note.contains("paused"));
    }

    #[test]
    fn local_ai_off_flags_summary_tree_precondition() {
        let _g = super::super::test_guard();
        let (_tmp, mut cfg) = test_config();
        cfg.embeddings_provider = Some("ollama:bge-m3".into()); // embeddings ok
        cfg.local_ai.runtime_enabled = false; // but summary tree needs local AI

        let report = run_doctor(&cfg);
        assert!(!report.healthy);
        // Embeddings ok, so the first blocking cause is the summary-tree stage.
        assert_eq!(
            report.first_blocking_cause.map(|f| f.code),
            Some(FailureCode::LocalModelUnavailable)
        );
        let tree = report.stages.iter().find(|s| s.stage == "summary_tree").unwrap();
        assert!(!tree.ok);
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
