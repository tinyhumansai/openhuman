//! Sanitized terminal module reports with deduplication for cached outcomes.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use super::types::ModuleRecord;

/// Closed reason vocabulary: never accept a module error, path or payload here.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Reason {
    UnknownModule,
    Disabled,
    LoaderDisabled,
    ResolutionFailed,
    IncompatibleContract,
    TransportFailed,
    ModuleFault,
    ModuleUnavailable,
}

impl Reason {
    fn code(self) -> &'static str {
        match self {
            Self::UnknownModule => "unknown_module",
            Self::Disabled => "disabled",
            Self::LoaderDisabled => "loader_disabled",
            Self::ResolutionFailed => "resolution_failed",
            Self::IncompatibleContract => "incompatible_contract",
            Self::TransportFailed => "transport_failed",
            Self::ModuleFault => "module_fault",
            Self::ModuleUnavailable => "module_unavailable",
        }
    }

    fn stage(self) -> &'static str {
        match self {
            Self::UnknownModule => "registry",
            Self::Disabled | Self::LoaderDisabled => "availability",
            Self::ResolutionFailed => "resolve",
            Self::IncompatibleContract => "contract",
            Self::TransportFailed => "transport",
            Self::ModuleFault => "execution",
            Self::ModuleUnavailable => "execution",
        }
    }
}

/// Report a terminal outcome once. Registry records supply all identifying tags.
///
/// There are finitely many registry records and reasons, so the process cache
/// is bounded. Concurrent callers cannot duplicate the same terminal report.
pub(super) fn report(record: &'static ModuleRecord, reason: Reason) {
    report_metadata(record.id, record.version, reason, true);
}

/// Report a failure from this invocation. Independent executions can fail
/// with the same sanitized reason and still represent distinct terminal events.
pub(super) fn report_invocation(record: &'static ModuleRecord, reason: Reason) {
    report_metadata(record.id, record.version, reason, false);
}

/// Unknown identifiers never become event metadata or deduplication keys.
pub(super) fn report_unknown_module() {
    report_metadata("unregistered", "unknown", Reason::UnknownModule, true);
}

fn report_metadata(module: &'static str, version: &'static str, reason: Reason, deduplicate: bool) {
    static REPORTED: OnceLock<Mutex<HashSet<(&'static str, Reason)>>> = OnceLock::new();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let capture = || {
        crate::core::observability::report_error(
            "loadable module operation failed",
            "modules",
            reason.stage(),
            &[
                ("module", module),
                ("version", version),
                ("stage", reason.stage()),
                ("platform", &platform),
                ("reason_code", reason.code()),
            ],
        )
    };
    // Pre-core embedders may call before Sentry is initialized. Do not spend
    // the process deduplication key on an event no bound client can receive.
    #[cfg(feature = "crash-reporting")]
    let retain_key = sentry::Hub::current().client().is_some();
    #[cfg(not(feature = "crash-reporting"))]
    let retain_key = true;
    if retain_key && deduplicate {
        let first = REPORTED
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((module, reason));
        if !first {
            return;
        }
    }
    // A host may have put request payloads or user paths on its current scope.
    // These terminal events carry only the closed metadata above.
    #[cfg(feature = "crash-reporting")]
    sentry::with_scope(|scope| scope.clear(), capture);
    #[cfg(not(feature = "crash-reporting"))]
    capture();
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
