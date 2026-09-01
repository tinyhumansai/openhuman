//! The doctor report and the degradation snapshot, read from the **driver**.
//!
//! # What moved, and what deliberately did not (#5560)
//!
//! `tinymemory_core::tree::health` answered both of these in-process:
//! `current_degraded_state()` read three atomics and `async_run_doctor()`
//! wrapped a blocking pass over the routing config, the scheduler gate, the job
//! queue and the chunk table. Neither is answerable from outside the engine —
//! the degradation flags are set by the embed and extract stages as they run,
//! and the counters are a read of the driver's own database — so both are
//! contract members now: `MemoryMaintenance::degraded_state` and
//! `MemoryMaintenance::diagnose`.
//!
//! That was not a cosmetic move. The flags are **process statics**, and the
//! loaded module links its own copy of `tinymemory-core`: a degradation the
//! module observed never reached the statics this host was reading, so
//! `current_degraded_state()` answered all-clear here forever. Asking the
//! driver is what makes the answer true again.
//!
//! # The response types are host-side, and their serde shape is pinned
//!
//! [`DoctorReport`], [`StageHealth`] and [`DoctorCounters`] are field-for-field
//! the engine's, including the two `skip_serializing_if`s and the one
//! `#[serde(default)]` **without** one — `DoctorCounters::extraction_coverage`
//! is serialised as `null` rather than omitted, which the contract's
//! `DiagnosisCounters` does not do. `report_tests` pins every one of those
//! against the JSON this RPC has always emitted.
//!
//! The failure taxonomy is **not** redefined here. [`PipelineFailure`],
//! [`FailureCode`] and [`FailureClass`] come from
//! [`super::taxonomy`](super) — the same items
//! `pipeline_status.first_blocking_cause` is built from out of a
//! `QueueFailure`. Two copies of that vocabulary is exactly how the two halves
//! of one response start disagreeing about a code. (Those types were the
//! engine's until #5560 and are this host's now; the sentence above was true
//! before the move for the same reason it is true after it — there is one
//! definition, wherever it lives.)

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::diagnosis::{
    DegradedCapabilities, Diagnosis, DiagnosisFailure,
};
use crate::openhuman::memory::api::provider::MemoryProvider;

use super::{DegradedState, FailureClass, FailureCode, PipelineFailure};

/// Health of one named pipeline stage.
///
/// The stage set is the **driver's** — a second engine has different stages —
/// and the order is meaningful: the stages run in it, so the first unhealthy
/// one is the one to fix first. Rendered as given rather than sorted.
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
    ///
    /// `#[serde(default)]` and **no** `skip_serializing_if`: the field is
    /// emitted as `null`, which is what this RPC has always sent and what the
    /// CLI and the agent tool read.
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

/// Carry one of the driver's classified failures across as this host's typed
/// one.
///
/// [`DiagnosisFailure::code`] and `class` are open-vocabulary strings on the
/// contract, and they are exactly the snake_case spellings [`FailureCode`] and
/// [`FailureClass`] serialise to — the driver builds them with the engine's own
/// `as_str`. So this is a parse, not a translation.
///
/// A code this build has no variant for yields `None`, which is the rule
/// `pipeline_status`'s own `blocking_cause` already applies to a queue
/// failure's reason: a cause this host cannot name is a cause it cannot render,
/// and inventing a nearest-wrong-variant would put the wrong remediation text
/// in front of the user.
///
/// `remediation_key` is taken from the driver rather than re-derived from the
/// code. The two agree for this engine — both come from
/// `FailureCode::remediation_key` — and the driver's is the authority for one
/// that retunes.
fn pipeline_failure(failure: &DiagnosisFailure) -> Option<PipelineFailure> {
    let code = FailureCode::from_str(&failure.code)?;
    let mut out = PipelineFailure::new(code);
    // Trust the driver's class when it stated one and this build knows the
    // spelling; otherwise keep the class derived from the code, which is what
    // an engine that classifies without deciding a retry policy leaves absent.
    match failure.class.as_deref() {
        Some("transient") => out.class = FailureClass::Transient,
        Some("unrecoverable") => out.class = FailureClass::Unrecoverable,
        _ => {}
    }
    if !failure.remediation_key.is_empty() {
        out.remediation_key = failure.remediation_key.clone();
    }
    // Already bounded driver-side by the same truncation this host would apply,
    // so it is carried rather than re-truncated.
    out.detail = failure.detail.clone();
    Some(out)
}

/// The contract's degradation snapshot as this host's [`DegradedState`].
///
/// Three booleans and at most one cause, one for one. `DegradedCapabilities`
/// is open-vocabulary only in its `cause`; the flags themselves are the same
/// three the engine set.
fn degraded_state_from(capabilities: &DegradedCapabilities) -> DegradedState {
    DegradedState {
        semantic_recall: capabilities.semantic_recall,
        structure: capabilities.structure,
        storage: capabilities.storage,
        cause: capabilities.cause.as_ref().and_then(pipeline_failure),
    }
}

/// Which capabilities the bound driver is currently running in a reduced mode.
///
/// # Errors
///
/// Whatever the driver's read failed with, and a binding that cannot be
/// resolved. A driver with no `Maintenance` family is **not** an error: it
/// reports no degradation, matching this RPC's other family reads
/// (`store_stats`, `queue_stats`, `backfill_in_progress`), which all answer
/// empty rather than failing a surface whose job is to describe the store.
pub async fn current_degraded_state(config: &Config) -> Result<DegradedState, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let guard = binding.guard();
    let Some(maintenance) = guard.as_maintenance() else {
        log::debug!(
            "[memory-tree][health] degraded_state: driver '{}' does not serve Maintenance; \
             reporting no degradation",
            binding.driver_id()
        );
        return Ok(DegradedState::default());
    };
    maintenance
        .degraded_state()
        .await
        .map(|capabilities| degraded_state_from(&capabilities))
        .map_err(|error| format!("degraded_state: {error}"))
}

/// The one-shot pipeline diagnostic, run by the bound driver.
///
/// Infallible by construction, the way the engine's `async_run_doctor` was: the
/// callers are an agent tool and an RPC that both owe the user a report, and a
/// diagnostic that refuses to answer is the least useful failure mode
/// available. A driver that cannot be reached, does not serve `Maintenance`, or
/// fails the call yields [`unavailable`] — the same shape the engine produced
/// when its own blocking task died.
pub async fn run_doctor(config: &Config) -> DoctorReport {
    let binding = match crate::openhuman::memory::binding::for_config(config) {
        Ok(binding) => binding,
        Err(error) => return unavailable(&format!("memory binding unavailable: {error}")),
    };
    let guard = binding.guard();
    let Some(maintenance) = guard.as_maintenance() else {
        return unavailable(&format!(
            "driver '{}' does not serve Maintenance",
            binding.driver_id()
        ));
    };
    match maintenance.diagnose().await {
        Ok(diagnosis) => doctor_report(diagnosis),
        Err(error) => {
            log::warn!("[memory-tree][health] doctor: diagnosis failed: {error}");
            unavailable(&error.to_string())
        }
    }
}

/// The driver's [`Diagnosis`] in the shape this RPC has always returned.
fn doctor_report(diagnosis: Diagnosis) -> DoctorReport {
    DoctorReport {
        healthy: diagnosis.healthy,
        stages: diagnosis
            .stages
            .into_iter()
            .map(|stage| StageHealth {
                stage: stage.stage,
                ok: stage.ok,
                failure: stage.failure.as_ref().and_then(pipeline_failure),
                note: stage.note,
            })
            .collect(),
        first_blocking_cause: diagnosis
            .first_blocking_cause
            .as_ref()
            .and_then(pipeline_failure),
        degraded: degraded_state_from(&diagnosis.degraded),
        counters: DoctorCounters {
            total_chunks: diagnosis.counters.total_chunks,
            jobs_ready: diagnosis.counters.jobs_ready,
            jobs_running: diagnosis.counters.jobs_running,
            jobs_failed: diagnosis.counters.jobs_failed,
            extraction_coverage: diagnosis.counters.extraction_coverage,
        },
    }
}

/// The report for a diagnosis that could not be run at all.
///
/// `healthy: false` with a transient cause and no stages, which is what the
/// engine's own `async_run_doctor` answered when its blocking task died — a
/// shaped report rather than an error, because every caller expects one. The
/// degradation snapshot is left at its default: the flags belong to the same
/// driver, so one that could not answer the diagnosis cannot answer them
/// either, and a second round trip on a failure path would only fail again.
fn unavailable(reason: &str) -> DoctorReport {
    DoctorReport {
        healthy: false,
        stages: Vec::new(),
        // The reason rides `detail`, which is the field that exists for exactly
        // this — non-localised, operator-facing, never a secret. The engine's
        // fallback carried none, so the surfaces that resolve `remediation_key`
        // are unaffected either way.
        first_blocking_cause: Some(
            PipelineFailure::new(FailureCode::Transient).with_detail(reason),
        ),
        degraded: DegradedState::default(),
        counters: DoctorCounters::default(),
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
