//! Tests for the Google Calendar default-args transformer in [`super`].
//!
//! See `googlecalendar_args.rs` for the bug background (#1714 — events
//! stored against a non-requester zone slipped outside the query window
//! because we shipped no `timeZone` / `singleEvents` defaults).

use serde_json::json;

use super::{apply_calendar_query_defaults, current_iana_timezone, TZ_DEFAULTING_SLUGS};

#[test]
fn injects_tz_and_single_events_when_absent() {
    let args = apply_calendar_query_defaults(
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(json!({
            "connectionId": "conn-1",
            "timeMin": "2026-05-14T00:00:00+05:30",
            "timeMax": "2026-05-14T23:59:59+05:30",
            "maxResults": 20
        })),
        "Asia/Kolkata",
    )
    .expect("transformer must return a payload");

    assert_eq!(args["timeZone"], "Asia/Kolkata");
    assert_eq!(args["singleEvents"], true);
    assert_eq!(
        args["connectionId"], "conn-1",
        "must not perturb caller-supplied fields"
    );
}

#[test]
fn does_not_overwrite_caller_supplied_time_zone() {
    // Power-user / heartbeat path may pass an explicit zone that
    // differs from the host. We must respect it, not silently swap to
    // host zone — the caller has more context than the host clock.
    let args = apply_calendar_query_defaults(
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(json!({
            "timeZone": "America/Los_Angeles",
            "timeMin": "2026-05-14T00:00:00-07:00",
        })),
        "Asia/Kolkata",
    )
    .unwrap();

    assert_eq!(args["timeZone"], "America/Los_Angeles");
    assert_eq!(args["singleEvents"], true);
}

#[test]
fn does_not_overwrite_caller_supplied_single_events() {
    let args = apply_calendar_query_defaults(
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(json!({
            "singleEvents": false,
        })),
        "UTC",
    )
    .unwrap();

    assert_eq!(args["singleEvents"], false);
    assert_eq!(args["timeZone"], "UTC");
}

#[test]
fn none_args_become_object_with_defaults() {
    let args = apply_calendar_query_defaults("GOOGLECALENDAR_EVENTS_LIST", None, "Europe/London")
        .expect("None must coerce to a populated default object");

    assert!(args.is_object());
    assert_eq!(args["timeZone"], "Europe/London");
    assert_eq!(args["singleEvents"], true);
}

#[test]
fn non_calendar_slug_is_untouched() {
    // The transformer must be a no-op for any slug not in the
    // allowlist — we never want to inject TZ semantics into, say,
    // gmail/notion/github args.
    let original = json!({
        "to": "alice@example.com",
        "subject": "hi",
    });
    let passed =
        apply_calendar_query_defaults("GMAIL_SEND_EMAIL", Some(original.clone()), "Asia/Kolkata")
            .unwrap();

    assert_eq!(passed, original);
}

#[test]
fn non_object_payload_is_untouched() {
    // Defensive: if a caller hands us a stray scalar/array, don't
    // synthesize an object — that would mask a real bug at the call
    // site. Pass it through and let the backend's schema check reject.
    let passed = apply_calendar_query_defaults(
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(json!(["unexpected"])),
        "Asia/Kolkata",
    )
    .unwrap();

    assert_eq!(passed, json!(["unexpected"]));
}

#[test]
fn slug_allowlist_covers_known_window_taking_actions() {
    // Sanity-check the allowlist content. EVENTS_LIST is the
    // user-visible "today/tomorrow" entry point; FIND_EVENT is the
    // sibling slug the model also picks for natural-language queries.
    assert!(TZ_DEFAULTING_SLUGS.contains(&"GOOGLECALENDAR_EVENTS_LIST"));
    assert!(TZ_DEFAULTING_SLUGS.contains(&"GOOGLECALENDAR_FIND_EVENT"));
}

#[test]
fn current_iana_timezone_returns_non_empty_string() {
    // Behavioural contract: caller can always treat the return value
    // as a non-empty `timeZone` value. Empty would break Google
    // Calendar's validation.
    let tz = current_iana_timezone();
    assert!(!tz.is_empty(), "iana lookup must fall back to a value");
}
