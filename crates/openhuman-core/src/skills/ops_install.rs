//! URL-based skill installation: fetch, validate, and write SKILL.md from a remote URL.

#[cfg(test)]
#[path = "ops_install_install_fetch_tests_tests.rs"]
mod install_fetch_tests;

mod fetch;
mod scan_gate;
mod uninstall;
mod url_validation;

pub(crate) use fetch::{install_validated_document, report_install_fetch_failure};
pub use fetch::{
    install_workflow_from_url, InstallWorkflowFromUrlOutcome, InstallWorkflowFromUrlParams,
    DEFAULT_INSTALL_TIMEOUT_SECS, MAX_INSTALL_TIMEOUT_SECS, MAX_WORKFLOW_MD_BYTES,
    RATE_LIMITED_ERROR_PREFIX,
};
pub(crate) use scan_gate::{fetch_scanned, gate_install};
pub use scan_gate::{
    ScanAcknowledgement, ScanBlockedOutcome, ScanFindingSummary, SkillInstallOutcome,
};
pub use uninstall::{uninstall_workflow, UninstallWorkflowOutcome, UninstallWorkflowParams};
pub(crate) use url_validation::{allow_local_http, ALLOW_LOCAL_HTTP_ENV};
pub use url_validation::{validate_install_url, validate_resolved_host, MAX_INSTALL_URL_LEN};

#[cfg(test)]
pub(crate) use fetch::{install_workflow_from_url_with_home, should_report_install_fetch_status};
#[cfg(test)]
pub(crate) use url_validation::normalize_install_url;
