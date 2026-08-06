//! Default-args transformer for Composio Google Calendar list/find slugs.
//!
//! Background (issue #1714): when the agent asks Composio for calendar
//! events on behalf of the user, the per-call argument object is built
//! either inside the agent loop (model-constructed) or inside the
//! heartbeat planner (`heartbeat/planner/collectors.rs`). Both paths
//! historically went out with UTC-encoded `timeMin` / `timeMax` and no
//! `timeZone` / `singleEvents` field.
//!
//! Two practical consequences for users in non-UTC timezones (real
//! report: IST user with an event stored against `Asia/Riyadh` (GST)):
//!
//! 1. Without `singleEvents = true`, the Google Calendar API returns the
//!    parent recurring-event entry, not the individual day's occurrence
//!    — and our downstream filter is keyed on the occurrence's
//!    `start.dateTime`. The user's recurring 12 PM IST stand-up never
//!    shows up because the response is the never-occurring template.
//!
//! 2. Without `timeZone`, the API normalises returned `start.dateTime`
//!    / `end.dateTime` against the *calendar's* default zone, which is
//!    not necessarily the requester's local zone. Our display + window
//!    filter both expect the requester's local zone, so events stored
//!    against a different zone slide outside the window.
//!
//! The fix is to default both fields at the *execute* boundary so the
//! agent's prompt template, the heartbeat planner, and any future
//! direct call site all benefit — no per-call-site discipline required.
//! Callers that pass either field explicitly win; we never overwrite
//! user-supplied intent.

use serde_json::Value;

/// Composio action slugs whose arguments accept (and benefit from)
/// `timeZone` + `singleEvents` defaulting. Kept short on purpose: only
/// the slugs that take a `timeMin`/`timeMax` window go in here.
///
/// Adding a new slug is the right move when (a) it accepts both fields
/// and (b) at least one production call site has been observed to fire
/// it without an explicit zone in production. Speculative additions
/// just expand the transformer's blast radius for zero current win.
pub(crate) const TZ_DEFAULTING_SLUGS: &[&str] =
    &["GOOGLECALENDAR_EVENTS_LIST", "GOOGLECALENDAR_FIND_EVENT"];

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

/// If `slug` is one of the [`TZ_DEFAULTING_SLUGS`] and `arguments` is a
/// JSON object, insert `singleEvents = true` and `timeZone = <iana>` —
/// but only when those keys are absent. Returns the (possibly
/// mutated) argument value.
///
/// Non-object payloads pass through untouched. The caller may pass
/// `None`/`null` to mean "no args supplied yet"; we treat that as an
/// empty object so the defaults still apply.
pub(crate) fn apply_calendar_query_defaults(
    slug: &str,
    arguments: Option<Value>,
    iana: &str,
) -> Option<Value> {
    if !TZ_DEFAULTING_SLUGS.contains(&slug) {
        tracing::debug!(
            target: "composio",
            slug,
            "[composio][googlecalendar] slug not in tz-defaulting allowlist; pass-through"
        );
        return arguments;
    }
    // Convert `None` to an empty object — the Composio backend treats
    // missing args + `{}` identically, so this just gives us a place to
    // hang our defaults without changing observable behaviour for
    // existing no-arg callers.
    let synthesised_object = arguments.is_none();
    let mut value = arguments.unwrap_or_else(|| Value::Object(Default::default()));
    let Some(map) = value.as_object_mut() else {
        tracing::debug!(
            target: "composio",
            slug,
            "[composio][googlecalendar] non-object payload; pass-through unchanged"
        );
        return Some(value);
    };
    let injected_time_zone = !map.contains_key("timeZone");
    if injected_time_zone {
        map.insert("timeZone".to_string(), Value::String(iana.to_string()));
    }
    let injected_single_events = !map.contains_key("singleEvents");
    if injected_single_events {
        map.insert("singleEvents".to_string(), Value::Bool(true));
    }
    tracing::debug!(
        target: "composio",
        slug,
        iana,
        synthesised_object,
        injected_time_zone,
        injected_single_events,
        "[composio][googlecalendar] applied calendar query defaults"
    );
    Some(value)
}

#[cfg(test)]
#[path = "googlecalendar_args_tests.rs"]
mod tests;
