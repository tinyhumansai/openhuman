//! The host's one redaction entry point for synchronous callers, answered by
//! the TinySecurity module.
//!
//! The module owns what text and endpoints may reveal (`Text`, `Endpoint`,
//! `EndpointOrigin`, `EndpointHost`): credentials, queries and fragments never
//! survive endpoint rendering.
//! These helpers exist because `tracing` fields, durable audit rows and
//! artifact writers are synchronous; they go through the shared sync seam and
//! answer the suppression marker, never the original text, when the module
//! cannot (not armed yet, faulted, timed out, or built without
//! `security-module`). Async callers use `modules::security::text` directly.

/// `scheme://host[:port]` of `url`, without credentials, path, query or fragment.
#[must_use]
pub fn endpoint_origin(url: &str) -> String {
    redact(Kind::Origin, url)
}

/// URL with credentials, query and fragment removed while preserving its safe
/// path, for diagnostics and install output that benefit from endpoint context.
#[must_use]
pub fn endpoint(url: &str) -> String {
    redact(Kind::Endpoint, url)
}

/// Hostname of `url` only (no scheme, port, credentials or path).
#[must_use]
pub fn endpoint_host(url: &str) -> String {
    redact(Kind::Host, url)
}

/// Secrets and PII removed from free `text` (the module's `Text` mode), for
/// callers that persist or show the result: audit rows, halt summaries,
/// failure lines and artifact bodies. Waits up to [`TEXT_TIMEOUT`] for the
/// module.
///
/// If the module cannot answer, the complete value is suppressed. The fixed
/// diagnostic floor is reserved for log fields and never authorizes persisted
/// or model-visible content.
#[must_use]
pub fn text(text: &str) -> String {
    answer_or_suppress(module_text(text))
}

/// Structured tool-call arguments with content, credentials and home paths
/// removed, for callers that persist or show them: approval prompts and rows,
/// flow audit records. The module's `StructuredArguments` mode keeps keys and
/// ordinary scalars, replaces content fields (`body`, `subject`, `to`, ...) with
/// a count marker such as `<redacted: string (12 chars)>` and credential fields
/// with `[REDACTED]`, and cleans every remaining string.
///
/// Fail closed: when the module cannot answer (not armed, faulted, timed out,
/// built without `security-module`) or answers with something that is not JSON,
/// the arguments are not shown at all; the result is a single marker that
/// carries only the value's shape. There is no host-side structured redactor to
/// fall back to, and the original value is never returned.
#[must_use]
pub fn arguments(args: &serde_json::Value) -> serde_json::Value {
    arguments_from(module_arguments(args), args)
}

/// Credentials removed from a tool result, or `None` when it must be withheld.
///
/// This is the module's `CredentialsOnly` mode: provider token shapes, JWTs,
/// PEM keys, cookie headers and labelled values that look like credentials are
/// replaced; PII and source code that merely talks about tokens are kept, so a
/// file the user asked the model to read arrives intact (#6954). The module
/// bounds one request (64 KiB), so a larger result is answered in line-aligned
/// chunks of [`CREDENTIAL_CHUNK_BYTES`]. A single line longer than a chunk is
/// split on a char boundary, so a secret straddling that split is not seen
/// whole; real credentials are far shorter than a chunk and ordinary text has
/// line breaks.
///
/// Fail closed whenever the module cannot answer, including when it is not
/// built or has not been armed. The fixed diagnostic floor is for logs only;
/// it does not authorize model-visible tool output.
#[must_use]
pub fn credentials(text: &str) -> Option<String> {
    credentials_with(text, |chunk| module_credentials(chunk).flatten())
}

/// Largest slice handed to the module in one `CredentialsOnly` call.
pub const CREDENTIAL_CHUNK_BYTES: usize = 48 * 1024;

/// `text` cut into slices of at most [`CREDENTIAL_CHUNK_BYTES`], on line breaks
/// where there are any (see [`credentials`]). Concatenating the slices gives
/// `text` back byte for byte.
pub(crate) fn credential_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(CREDENTIAL_CHUNK_BYTES);
        if end < rest.len() {
            // Prefer the last line break inside the window.
            if let Some(nl) = rest[..end].rfind('\n') {
                end = nl + 1;
            } else {
                while !rest.is_char_boundary(end) {
                    end -= 1;
                }
            }
        }
        chunks.push(&rest[..end]);
        rest = &rest[end..];
    }
    chunks
}

/// [`credentials`] over an explicit per-chunk answerer, so the chunking is
/// testable without a module.
pub(crate) fn credentials_with(
    text: &str,
    mut answer: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    if text.len() <= CREDENTIAL_CHUNK_BYTES {
        return answer(text);
    }
    let mut out = String::with_capacity(text.len());
    for chunk in credential_chunks(text) {
        out.push_str(&answer(chunk)?);
    }
    Some(out)
}

/// `Some(answer)` when a module is serving (the answer itself may be `None`:
/// it failed, so the result is withheld), `None` when no module serves and the
/// floor applies.
#[cfg(feature = "security-module")]
fn module_credentials(chunk: &str) -> Option<Option<String>> {
    use crate::modules::security::text::{bridge_armed, try_redact, RedactMode};
    if !bridge_armed() {
        return None;
    }
    Some(
        match try_redact(RedactMode::CredentialsOnly, chunk, Some(TEXT_TIMEOUT)) {
            Ok(redacted) => Some(redacted),
            Err(error) => {
                log::debug!(
                    "[security][redact] module credential pass unavailable, withheld: {error}"
                );
                None
            }
        },
    )
}

#[cfg(not(feature = "security-module"))]
fn module_credentials(_chunk: &str) -> Option<Option<String>> {
    None
}

/// The module's `answer` as JSON, or the shape-only marker for `args`.
fn arguments_from(answer: Option<String>, args: &serde_json::Value) -> serde_json::Value {
    answer
        .and_then(|redacted| serde_json::from_str(&redacted).ok())
        .unwrap_or_else(|| unavailable_marker(args))
}

/// What replaces arguments the module could not redact: the kind and size only.
fn unavailable_marker(args: &serde_json::Value) -> serde_json::Value {
    let shape = match args {
        serde_json::Value::Object(map) => format!("object ({} keys)", map.len()),
        serde_json::Value::Array(items) => format!("array ({} items)", items.len()),
        serde_json::Value::String(text) => format!("string ({} chars)", text.chars().count()),
        serde_json::Value::Null => return serde_json::Value::Null,
        _ => "scalar".to_string(),
    };
    serde_json::Value::String(format!("<redacted: {shape}; arguments unavailable>"))
}

#[cfg(feature = "security-module")]
fn module_arguments(args: &serde_json::Value) -> Option<String> {
    use crate::modules::security::text::{try_redact, RedactMode};
    let input = serde_json::to_string(args).ok()?;
    match try_redact(RedactMode::StructuredArguments, &input, Some(TEXT_TIMEOUT)) {
        Ok(redacted) => Some(redacted),
        Err(error) => {
            log::debug!("[security][redact] module argument pass unavailable, withheld: {error}");
            None
        }
    }
}

#[cfg(not(feature = "security-module"))]
fn module_arguments(_args: &serde_json::Value) -> Option<String> {
    None
}

/// The module's `answer` when it gave one, otherwise the suppression marker.
fn answer_or_suppress(answer: Option<String>) -> String {
    answer.unwrap_or_else(|| crate::core::log_redaction::SUPPRESSED.to_string())
}

#[cfg(feature = "security-module")]
fn module_text(input: &str) -> Option<String> {
    use crate::modules::security::text::{try_redact, RedactMode};
    match try_redact(RedactMode::Text, input, Some(TEXT_TIMEOUT)) {
        Ok(redacted) => Some(redacted),
        Err(error) => {
            log::debug!(
                "[security][redact] module text pass unavailable, content suppressed: {error}"
            );
            None
        }
    }
}

#[cfg(not(feature = "security-module"))]
fn module_text(_input: &str) -> Option<String> {
    None
}

/// Longest [`text`] waits for the module; large artifact bodies need more than
/// a log line does.
const TEXT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

enum Kind {
    Endpoint,
    Origin,
    Host,
}

#[cfg(feature = "security-module")]
fn redact(kind: Kind, input: &str) -> String {
    use crate::modules::security::text::{redact_for_log, RedactMode};
    let mode = match kind {
        Kind::Endpoint => RedactMode::Endpoint,
        Kind::Origin => RedactMode::EndpointOrigin,
        Kind::Host => RedactMode::EndpointHost,
    };
    redact_for_log(mode, input, None)
}

#[cfg(not(feature = "security-module"))]
fn redact(_kind: Kind, _input: &str) -> String {
    crate::core::log_redaction::SUPPRESSED.to_string()
}

#[cfg(test)]
#[path = "redact_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "redact_arguments_tests.rs"]
mod arguments_tests;
