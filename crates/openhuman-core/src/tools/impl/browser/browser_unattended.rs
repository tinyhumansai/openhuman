//! Opt-in pass for allow-listed browser actions in trusted unattended turns.
//!
//! Every consequential browser action asks the forced host approval gate,
//! which parks only for a routable WebChat turn and denies everything else.
//! A cron job, background job or approval-free workflow has nobody to answer,
//! so it could not even follow a "next page" link. `[browser]
//! unattended_actions` names the gated kinds such a turn may take without
//! asking; every other origin keeps the forced gate exactly as it was.

use crate::config::BrowserConfig;
use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

/// A turn the user authorized ahead of time and nobody is watching live.
/// A workflow that asked for review (`require_approval`) is excluded.
fn is_unattended(origin: &AgentTurnOrigin) -> bool {
    matches!(
        origin,
        AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Cron
                | TrustedAutomationSource::Background
                | TrustedAutomationSource::Workflow {
                    require_approval: false
                },
            ..
        }
    )
}

/// The canonical name of `kind` when a turn with `origin` may take it
/// without the approval gate.
fn unattended_kind(
    origin: Option<&AgentTurnOrigin>,
    browser: &BrowserConfig,
    kind: &str,
) -> Option<&'static str> {
    origin
        .is_some_and(is_unattended)
        .then(|| browser.unattended_kind(kind))
        .flatten()
}

/// Whether a turn with `origin` may take `kind` without the approval gate.
pub(super) fn allowed_for(
    origin: Option<&AgentTurnOrigin>,
    browser: &BrowserConfig,
    kind: &str,
) -> bool {
    unattended_kind(origin, browser, kind).is_some()
}

/// Decide for the turn `origin` the tool was called under (read once from the
/// per-turn `CoreContext` in `BrowserTool::execute`), logging an allowed
/// action by its canonical static kind and digest only: no caller-supplied
/// string, selector, typed value or page content reaches the log.
pub(super) fn allow(
    origin: Option<&AgentTurnOrigin>,
    browser: &BrowserConfig,
    kind: &str,
    digest_hex: &str,
) -> bool {
    let Some(canonical) = unattended_kind(origin, browser, kind) else {
        return false;
    };
    let digest = digest_hex
        .get(..12)
        .filter(|prefix| prefix.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or("invalid");
    tracing::info!(
        action = canonical,
        action_digest = digest,
        origin = %origin.map(AgentTurnOrigin::class).unwrap_or_default(),
        "[browser] unattended action allowed by [browser] unattended_actions"
    );
    true
}

#[cfg(test)]
#[path = "browser_unattended_tests.rs"]
mod tests;
