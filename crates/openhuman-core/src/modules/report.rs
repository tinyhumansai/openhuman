//! Terminal module outcomes, reported to Sentry.
//!
//! Collecting, sanitising and deduplicating a report is `tinybus::module`'s
//! [`FailureReporter`]; this file supplies the one piece that is OpenHuman's own,
//! the sink that turns a report into a Sentry event, and the process-wide
//! reporter every resolver and client shares so a terminal outcome is reported
//! once per process however many clients observed it.

use std::sync::{Arc, OnceLock};

use tinybus::module::{
    classify, FailureReport, FailureReporter, FailureSink, ModuleCallError, Reason,
};

use super::registry;

/// Delivers a [`FailureReport`] as a Sentry event carrying only closed metadata.
struct SentrySink;

impl FailureSink for SentrySink {
    /// Pre-core embedders may report before Sentry is initialised. The reporter
    /// does not spend its deduplication key on an event no bound client can
    /// receive.
    fn ready(&self) -> bool {
        #[cfg(feature = "crash-reporting")]
        {
            sentry::Hub::current().client().is_some()
        }
        #[cfg(not(feature = "crash-reporting"))]
        {
            true
        }
    }

    fn capture(&self, report: &FailureReport<'_>) {
        let capture = || {
            crate::core::observability::report_error(
                "loadable module operation failed",
                "modules",
                report.reason.stage(),
                &[
                    ("module", report.module),
                    ("version", report.version),
                    ("stage", report.reason.stage()),
                    ("platform", report.platform),
                    ("reason_code", report.reason.code()),
                ],
            )
        };
        // A host may have put request payloads or user paths on its current
        // scope. These terminal events carry only the closed metadata above.
        #[cfg(feature = "crash-reporting")]
        sentry::with_scope(|scope| scope.clear(), capture);
        #[cfg(not(feature = "crash-reporting"))]
        capture();
    }
}

/// The reporter shared by every resolver and client in this process.
#[must_use]
pub(crate) fn reporter() -> Arc<FailureReporter> {
    static REPORTER: OnceLock<Arc<FailureReporter>> = OnceLock::new();
    Arc::clone(REPORTER.get_or_init(|| Arc::new(FailureReporter::new(Arc::new(SentrySink)))))
}

/// Report a structurally invalid reply from a known module invocation.
pub(crate) fn malformed_reply(module: &str) {
    if let Some(record) = registry::find(module) {
        reporter().report_invocation(record.id, record.version, Reason::ModuleFault);
    }
}

/// Report a registered module that advertises an older wire contract.
pub(crate) fn incompatible_contract(module: &str) {
    if let Some(record) = registry::find(module) {
        reporter().report(record.id, record.version, Reason::IncompatibleContract);
    }
}

/// Report a known module that does not provide the requested capability.
pub(crate) fn unavailable(module: &str) {
    if let Some(record) = registry::find(module) {
        reporter().report(record.id, record.version, Reason::ModuleUnavailable);
    }
}

/// Report a bus failure from a call made on a proxy the caller holds, without
/// exposing its payload, and return the sanitised outcome. A module that
/// reports itself unavailable is a cached, once-per-process event; every other
/// failure is a distinct invocation.
pub(crate) fn proxy_failure(
    record: &'static super::types::ModuleRecord,
    error: &tinybus::Error,
) -> ModuleCallError {
    let (outcome, reason) = classify(error);
    if reason == Reason::ModuleUnavailable {
        reporter().report(record.id, record.version, reason);
    } else {
        reporter().report_invocation(record.id, record.version, reason);
    }
    outcome
}

/// Report that the module bus itself could not be reached for `record`.
pub(crate) fn transport_failed(record: &'static super::types::ModuleRecord) {
    reporter().report(record.id, record.version, Reason::TransportFailed);
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
