//! Tests for the surrounding module.
//!
//! These pin **wire bytes**, not behaviour: the doctor report and the
//! degradation snapshot moved from the engine's types to this host's in #5560,
//! and the whole point of the move was that nothing downstream could tell.
//! Every JSON literal below is what the engine's `DoctorReport` /
//! `DegradedState` serialised to before the swap.

use super::*;
use crate::openhuman::memory::api::provider::diagnosis::{DiagnosisCounters, DiagnosisStage};

/// A healthy report: `first_blocking_cause` is **omitted** (the engine's
/// `skip_serializing_if`), `stages` is present and empty, and
/// `counters.extraction_coverage` is present as `null` — that last one is the
/// asymmetry with the contract's `DiagnosisCounters`, which skips it.
#[test]
fn healthy_report_serde_shape_is_unchanged() {
    let report = doctor_report(Diagnosis {
        healthy: true,
        stages: Vec::new(),
        first_blocking_cause: None,
        degraded: DegradedCapabilities::default(),
        counters: DiagnosisCounters::default(),
    });

    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::json!({
            "healthy": true,
            "stages": [],
            "degraded": {
                "semantic_recall": false,
                "structure": false,
                "storage": false,
            },
            "counters": {
                "total_chunks": 0,
                "jobs_ready": 0,
                "jobs_running": 0,
                "jobs_failed": 0,
                "extraction_coverage": null,
            },
        })
    );
}

/// An unhealthy one, with every optional field populated. `code` and `class`
/// are the snake_case enum spellings, `remediation_key` is verbatim, `detail`
/// appears only because it is `Some`, and the degradation `cause` nests the
/// same object.
#[test]
fn unhealthy_report_serde_shape_is_unchanged() {
    let failure = DiagnosisFailure {
        code: "embeddings_unconfigured".into(),
        class: Some("unrecoverable".into()),
        remediation_key: "memory.health.remediation.embeddings_unconfigured".into(),
        detail: Some("no provider".into()),
    };
    let report = doctor_report(Diagnosis {
        healthy: false,
        stages: vec![
            DiagnosisStage {
                stage: "storage".into(),
                ok: true,
                failure: None,
                note: "memory storage path is writable".into(),
            },
            DiagnosisStage {
                stage: "embeddings".into(),
                ok: false,
                failure: Some(failure.clone()),
                note: "no embeddings provider".into(),
            },
        ],
        first_blocking_cause: Some(failure.clone()),
        degraded: DegradedCapabilities {
            semantic_recall: true,
            structure: false,
            storage: false,
            cause: Some(failure),
        },
        counters: DiagnosisCounters {
            total_chunks: 4,
            jobs_ready: 1,
            jobs_running: 2,
            jobs_failed: 3,
            extraction_coverage: Some(0.25),
        },
    });

    let cause = serde_json::json!({
        "code": "embeddings_unconfigured",
        "class": "unrecoverable",
        "remediation_key": "memory.health.remediation.embeddings_unconfigured",
        "detail": "no provider",
    });
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::json!({
            "healthy": false,
            "stages": [
                {
                    "stage": "storage",
                    "ok": true,
                    "note": "memory storage path is writable",
                },
                {
                    "stage": "embeddings",
                    "ok": false,
                    "failure": cause,
                    "note": "no embeddings provider",
                },
            ],
            "first_blocking_cause": cause,
            "degraded": {
                "semantic_recall": true,
                "structure": false,
                "storage": false,
                "cause": cause,
            },
            "counters": {
                "total_chunks": 4,
                "jobs_ready": 1,
                "jobs_running": 2,
                "jobs_failed": 3,
                "extraction_coverage": 0.25,
            },
        })
    );
}

/// The degradation snapshot on its own — the object
/// `pipeline_status.degraded` carries. Its `cause` is omitted when absent, and
/// `storage` is always present (the engine's `#[serde(default)]` has no
/// `skip_serializing_if`).
#[test]
fn degraded_snapshot_serde_shape_is_unchanged() {
    let clear = degraded_state_from(&DegradedCapabilities::default());
    assert_eq!(
        serde_json::to_value(&clear).unwrap(),
        serde_json::json!({
            "semantic_recall": false,
            "structure": false,
            "storage": false,
        })
    );
    assert!(!clear.is_degraded());

    let degraded = degraded_state_from(&DegradedCapabilities {
        semantic_recall: false,
        structure: false,
        storage: true,
        cause: Some(DiagnosisFailure {
            code: "storage_unavailable".into(),
            class: None,
            remediation_key: "memory.health.remediation.storage_unavailable".into(),
            detail: None,
        }),
    });
    assert_eq!(
        serde_json::to_value(&degraded).unwrap(),
        serde_json::json!({
            "semantic_recall": false,
            "structure": false,
            "storage": true,
            "cause": {
                "code": "storage_unavailable",
                "class": "unrecoverable",
                "remediation_key": "memory.health.remediation.storage_unavailable",
            },
        })
    );
}

/// A class the driver did not state falls back to the one this build derives
/// from the code, and a code this build has no variant for drops the cause
/// entirely rather than rendering a nearest-wrong remediation.
#[test]
fn failure_parse_falls_back_on_class_and_refuses_an_unknown_code() {
    let derived = pipeline_failure(&DiagnosisFailure {
        code: "extraction_timeout".into(),
        class: None,
        remediation_key: "memory.health.remediation.extraction_timeout".into(),
        detail: None,
    })
    .expect("a known code parses");
    assert_eq!(derived.code, FailureCode::ExtractionTimeout);
    assert_eq!(derived.class, FailureClass::Transient);

    assert!(pipeline_failure(&DiagnosisFailure {
        code: "a_cause_this_build_has_never_heard_of".into(),
        class: Some("transient".into()),
        remediation_key: "whatever".into(),
        detail: None,
    })
    .is_none());
}

/// A diagnosis that could not be run answers with a report rather than an
/// error — the shape the engine's own blocking-task fallback produced, plus the
/// reason on `detail`.
#[test]
fn unavailable_report_is_shaped_not_empty() {
    let report = unavailable("driver 'null' does not serve Maintenance");
    assert!(!report.healthy);
    assert!(report.stages.is_empty());
    let cause = report.first_blocking_cause.expect("a transient cause");
    assert_eq!(cause.code, FailureCode::Transient);
    assert_eq!(cause.class, FailureClass::Transient);
    assert!(cause
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("does not serve Maintenance")));
    assert_eq!(report.degraded, DegradedState::default());
    assert_eq!(report.counters, DoctorCounters::default());
}
