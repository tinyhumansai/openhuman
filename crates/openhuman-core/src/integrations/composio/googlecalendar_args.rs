//! Host time-zone lookup for Google Calendar defaults.
//!
//! The defaulting itself (`timeZone` + `singleEvents` for calendar list/find
//! actions, issue #1714) is the connector module’s `PrepareArguments` operation.
//! It takes the zone as a parameter because reading the machine's zone is host
//! business; this is that lookup.

/// Resolve the host's IANA zone name (`Asia/Kolkata`, `America/Los_Angeles`).
/// Falls back to `"UTC"` when the host can't resolve a zone — e.g. CI
/// containers without `/etc/localtime`, or stripped Docker images. The
/// fall-back keeps the call site behaviour-equivalent to today (zone
/// implicitly UTC) rather than crashing.
pub(crate) fn current_iana_timezone() -> String {
    match iana_time_zone::get_timezone() {
        Ok(tz) => tz,
        Err(error) => {
            tracing::debug!(
                target: "composio",
                error = %error,
                "[composio][googlecalendar] iana_time_zone lookup failed; falling back to UTC"
            );
            "UTC".to_string()
        }
    }
}

#[cfg(test)]
#[path = "googlecalendar_args_tests.rs"]
mod tests;
