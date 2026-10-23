//! Host wiring for the harness `CredentialScrubMiddleware`: the scrub itself
//! is the TinySecurity module's `CredentialsOnly` redaction
//! (`security::redact::credentials`); the harness supplies the middleware and
//! the notice. The only host policy is the browser task's confirmation token,
//! which is minted by the host and must survive the scrubber.

use tinyagents_harness::middleware::{redaction_notice, CredentialScrubMiddleware, ToolScrubber};

/// What the module writes where it removed a credential.
const MODULE_PLACEHOLDER: &str = "[REDACTED]";

/// What a tool result becomes when the security module cannot check it: the
/// result is withheld whole. An unchecked result is never passed on.
pub(crate) const WITHHELD_NOTICE: &str = "[withheld: the security module could not check this tool result for credentials, so it was not passed on]";

/// The module's `CredentialsOnly` answer for `text`, or `None` when it cannot
/// answer.
pub(crate) type Redactor<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Count the values one pass replaced, not the placeholders the text already
/// carried (a result may legitimately contain one).
fn count_redactions(before: &str, after: &str) -> usize {
    after
        .matches(MODULE_PLACEHOLDER)
        .count()
        .saturating_sub(before.matches(MODULE_PLACEHOLDER).count())
}

/// Scrub `content` through `redact`: `None` when nothing changed, the scrubbed
/// text with the model-facing notice when something did, and the withheld
/// notice when the redactor could not answer (fail closed).
fn scrub_through(redact: Redactor<'_>, content: &str) -> Option<(String, usize)> {
    let Some(scrubbed) = redact(content) else {
        log::warn!("[credential_scrub] module unavailable; tool result withheld");
        return Some((WITHHELD_NOTICE.to_string(), 1));
    };
    if scrubbed == content {
        return None;
    }
    let redactions = count_redactions(content, &scrubbed);
    Some((
        format!("{scrubbed}\n\n{}", redaction_notice(redactions)),
        redactions,
    ))
}

/// The browser task's `pending.token` is a host-minted, one-time confirmation
/// handle, not a credential from page content. Protect only that field in a
/// `NeedsConfirmation` result; every other string still crosses the module's
/// credential scrubber, including page text and action input.
///
/// The token is removed before scrubbing and put back afterwards, so it is
/// never seen by (or counted against) the scrubber.
pub(crate) fn scrub_for_tool(
    redact: Redactor<'_>,
    tool_name: &str,
    content: &str,
) -> Option<(String, usize)> {
    if tool_name != "browser" {
        return scrub_through(redact, content);
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(content) else {
        return scrub_through(redact, content);
    };
    if value["status"] != "NeedsConfirmation" {
        return scrub_through(redact, content);
    }
    let Some(token) = value["pending"]["token"].as_str().map(str::to_owned) else {
        return scrub_through(redact, content);
    };
    if token.len() != 36 || uuid::Uuid::parse_str(&token).is_err() {
        return scrub_through(redact, content);
    }
    // Take the token out of the document instead of masking it with a dummy
    // value: the scrubber redacts secret-looking `token` values of any
    // length, so a UUID-shaped placeholder is redacted and counted too.
    if let Some(pending) = value["pending"].as_object_mut() {
        pending.remove("token");
    }
    let Ok(protected) = serde_json::to_string(&value) else {
        return scrub_through(redact, content);
    };
    let Some(scrubbed) = redact(&protected) else {
        log::warn!("[credential_scrub] module unavailable; tool result withheld");
        return Some((WITHHELD_NOTICE.to_string(), 1));
    };
    if scrubbed == protected {
        return None;
    }
    let redactions = count_redactions(&protected, &scrubbed);
    let Ok(mut result) = serde_json::from_str::<serde_json::Value>(&scrubbed) else {
        // The module changed the document into something that is not JSON;
        // the token cannot be put back, so scrub the original whole.
        return scrub_through(redact, content);
    };
    result["pending"]["token"] = serde_json::Value::String(token);
    Some((
        format!("{}\n\n{}", result, redaction_notice(redactions)),
        redactions,
    ))
}

/// The credential scrub wrap the turn harness installs innermost.
pub(crate) fn credential_scrub_middleware() -> CredentialScrubMiddleware {
    let scrubber: ToolScrubber = std::sync::Arc::new(|tool_name, content| {
        scrub_for_tool(&crate::security::redact::credentials, tool_name, content)
    });
    CredentialScrubMiddleware::with_scrubber(scrubber)
}

#[cfg(test)]
#[path = "credential_scrub_tests.rs"]
mod tests;
