//! Host wiring for the harness `CredentialScrubMiddleware`: the scrub itself
//! lives upstream; the only host policy is the browser task's confirmation
//! token, which is minted by the host and must survive the scrubber.

use tinyagents_harness::middleware::{
    redaction_notice, CredentialScrubMiddleware, ToolScrubber, REDACTION_PLACEHOLDER,
};

/// The browser task's `pending.token` is a host-minted, one-time confirmation
/// handle, not a credential from page content. Protect only that field in a
/// `NeedsConfirmation` result; every other string still crosses the ordinary
/// credential scrubber, including page text and action input.
///
/// The token is removed before scrubbing and put back afterwards, so it is
/// never seen by (or counted against) the scrubber.
fn scrub_with_notice_for_tool(_tool_name: &str, content: &str) -> Option<(String, usize)> {
    #[cfg(feature = "security-module")]
    let mode = if _tool_name == "browser"
        && serde_json::from_str::<serde_json::Value>(content)
            .ok()
            .is_some_and(|value| {
                value["status"] == "NeedsConfirmation"
                    && value["pending"]["token"].as_str().is_some_and(|token| {
                        token.len() == 36 && uuid::Uuid::parse_str(token).is_ok()
                    })
            }) {
        tinysecurity_bus::RedactMode::BrowserResult
    } else {
        tinysecurity_bus::RedactMode::CredentialsOnly
    };
    #[cfg(feature = "security-module")]
    let scrubbed = crate::modules::security::text::redact_blocking(
        mode,
        content,
        std::time::Duration::from_secs(2),
    );
    #[cfg(not(feature = "security-module"))]
    let scrubbed: Result<String, String> = Err("TinySecurity is not built into this host".into());
    let Ok(scrubbed) = scrubbed else {
        let count = 1;
        return Some((
            format!("{REDACTION_PLACEHOLDER}\n\n{}", redaction_notice(count)),
            count,
        ));
    };
    if scrubbed == content {
        return None;
    }
    let redactions = scrubbed
        .matches(REDACTION_PLACEHOLDER)
        .count()
        .saturating_sub(content.matches(REDACTION_PLACEHOLDER).count())
        .max(1);
    Some((
        format!("{scrubbed}\n\n{}", redaction_notice(redactions)),
        redactions,
    ))
}

/// The credential scrub wrap the turn harness installs innermost.
pub(crate) fn credential_scrub_middleware() -> CredentialScrubMiddleware {
    let scrubber: ToolScrubber = std::sync::Arc::new(scrub_with_notice_for_tool);
    CredentialScrubMiddleware::with_scrubber(scrubber)
}

#[cfg(test)]
#[path = "credential_scrub_tests.rs"]
mod tests;
