//! When a connector sync pass has failed, and how long a `composio_sync` run
//! waits before asking again (openhuman#6255).
//!
//! The connector module reports a page it could not read as
//! [`SyncStage::Failed`]: it keeps the pages it had already read, puts the
//! reason in the response's `message`, and leaves the batch incomplete. An
//! incomplete batch otherwise means "call again", so a connector that failed on
//! its first page was asked again at once, up to fifty times a click, with the
//! reason logged nowhere. These two decisions end that: a failed pass is a
//! failure, and the only retries are a few spaced ones.

use std::time::Duration;

use tinyconnectors_bus::records::{ConnectorSyncResponse, SyncStage};

/// Longest failure reason kept, in characters.
///
/// A provider error can carry a whole response body. The reason is shown on
/// the source row and written to the log, so it is cut rather than kept whole.
pub(crate) const MAX_REASON_CHARS: usize = 300;

/// Attempts a `composio_sync` run makes at a pass the connector keeps failing:
/// the first try and two retries.
pub(crate) const MAX_FAILED_ATTEMPTS: u32 = 3;

/// Why a pass failed, or `None` for a pass that did not.
///
/// Only the connector's own failed stage counts. Host-side errors (the module
/// being unavailable, a timed-out call, an ingest error) are already `Err` at
/// the call site and are not retried: repeating them cannot help, and a timed
/// out call may still be running.
///
/// The reason is the connector's message on one line, cut to
/// [`MAX_REASON_CHARS`]. A failure that gave no message is still named, so a
/// row never reads "Sync failed:" with nothing after it.
pub(crate) fn failure_reason(
    stage: SyncStage,
    message: Option<&str>,
    toolkit: &str,
) -> Option<String> {
    if stage != SyncStage::Failed {
        return None;
    }
    Some(
        message
            .map(one_line)
            .filter(|message| !message.is_empty())
            .map(clip)
            .unwrap_or_else(|| format!("the {toolkit} connector stopped without saying why")),
    )
}

/// The failure the connector reported for one sync pass, logged once.
///
/// See [`failure_reason`] for what counts. Logged here, where a pass's response
/// is read, so every caller gets the same line, with how far the pass got
/// before it stopped.
pub(crate) fn pass_failure(
    response: &ConnectorSyncResponse,
    toolkit: &str,
    connection_id: &str,
) -> Option<String> {
    let reason = failure_reason(response.stage, response.message.as_deref(), toolkit)?;
    tracing::warn!(
        toolkit = %toolkit,
        connection_id = %connection_id,
        pages_read = response.pages_read,
        records = response.batch.records.len(),
        reason = %reason,
        "[composio] connector sync pass failed"
    );
    Some(reason)
}

/// `message` with every run of whitespace, line breaks included, made one
/// space. A provider error can span lines (the complaint, then the fields it
/// wanted), and the reason is shown as one row and logged as one line.
fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `message` cut to at most [`MAX_REASON_CHARS`] characters, the ellipsis that
/// marks the cut included, on a character boundary.
fn clip(message: String) -> String {
    if message.chars().count() <= MAX_REASON_CHARS {
        return message;
    }
    let cut = message
        .char_indices()
        .nth(MAX_REASON_CHARS - 1)
        .map_or(message.len(), |(index, _)| index);
    format!("{}…", message[..cut].trim_end())
}

/// How long to wait before asking again after `failed_attempts` consecutive
/// failed passes, or `None` once the run should give up.
///
/// Five seconds after the first failure, ten after the second, and no third
/// retry: a transient provider error still recovers, and a connector that is
/// broken shows its failure within about fifteen seconds instead of holding
/// the row on "Syncing".
pub(crate) fn retry_delay(failed_attempts: u32) -> Option<Duration> {
    if failed_attempts == 0 || failed_attempts >= MAX_FAILED_ATTEMPTS {
        return None;
    }
    Some(Duration::from_secs(5 * 2_u64.pow(failed_attempts - 1)))
}

#[cfg(test)]
#[path = "pass_failure_tests.rs"]
mod tests;
