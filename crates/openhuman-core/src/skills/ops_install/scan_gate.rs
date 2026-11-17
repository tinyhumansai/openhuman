//! Host adapter over `tinyskills::scan_gate`.
//!
//! The acknowledgement, blocked-document summary, retry policy and install gate
//! live in the module. This file keeps the host's grep-friendly log lines
//! around those calls and names the install result with the host's
//! [`InstallWorkflowFromUrlOutcome`].

use std::future::Future;

use tinyskills::{RegistryDocument, RegistryError};

pub use tinyskills::{ScanAcknowledgement, ScanBlocked as ScanBlockedOutcome, ScanFindingSummary};

use super::fetch::InstallWorkflowFromUrlOutcome;

/// What an install did: installed the skill, or refused it on its scan.
pub type SkillInstallOutcome = tinyskills::Gated<InstallWorkflowFromUrlOutcome>;

/// Fetch and scan a document through the module's retry policy, logging each
/// outcome for `target`.
pub(crate) async fn fetch_scanned<F, Fut>(
    target: &str,
    acknowledgement: &ScanAcknowledgement,
    fetch: F,
) -> Result<RegistryDocument, RegistryError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RegistryDocument, RegistryError>>,
{
    let mut fetch = fetch;
    let mut attempts = 0usize;
    let result = tinyskills::fetch_scanned(acknowledgement, || {
        attempts += 1;
        fetch()
    })
    .await;
    match &result {
        Ok(document) if attempts > 1 => tracing::info!(
            install_target = %target,
            attempts,
            blocked = document.is_blocked(),
            digest = %document.digest,
            "[skills] scan gate: retry fetched the document"
        ),
        Ok(document) if document.is_blocked() => tracing::warn!(
            install_target = %target,
            fetched_from = %document.fetched_from,
            digest = %document.digest,
            findings = document.scan.findings.len(),
            "[skills] scan gate: blocked document installed on user acknowledgement"
        ),
        Ok(_) => {}
        Err(error) => tracing::info!(
            install_target = %target,
            attempts,
            kind = error.kind().as_str(),
            "[skills] scan gate: fetch failed"
        ),
    }
    result
}

/// Turn a scanned document into an install, or into a refusal when its scan
/// blocks and the user has not acknowledged this document's digest.
pub(crate) fn gate_install(
    target: &str,
    acknowledgement: &ScanAcknowledgement,
    document: RegistryDocument,
    install: impl FnOnce(RegistryDocument) -> Result<InstallWorkflowFromUrlOutcome, String>,
) -> Result<SkillInstallOutcome, String> {
    let outcome = tinyskills::gate_install(target, acknowledgement, document, install)?;
    if let SkillInstallOutcome::ScanBlocked(blocked) = &outcome {
        tracing::warn!(
            install_target = %target,
            fetched_from = %blocked.fetched_from,
            slug = %blocked.slug,
            digest = %blocked.digest,
            acknowledged = acknowledgement.is_given(),
            findings = blocked.findings.len(),
            "[skills] scan gate: refused install; scan still blocks and the document is not acknowledged"
        );
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "scan_gate_tests.rs"]
mod tests;
