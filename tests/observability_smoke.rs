//! Runtime smoke for the Sentry `before_send` filter that drops per-attempt
//! transient-upstream provider failures (OPENHUMAN-TAURI-2E / 84 / T).
//!
//! Unit tests in `src/core/observability.rs` exercise the pure filter
//! function. This integration test wires the actual `sentry::init` →
//! `before_send` → transport chain so we have proof the runtime path
//! behaves as designed: transient events are dropped, permanent events
//! and aggregate `all_exhausted` events still surface.

use openhuman_core::core::observability::is_transient_provider_http_failure;
use sentry::protocol::Event;
use std::collections::BTreeMap;
use std::sync::Arc;

fn event_with_tags(tags: &[(&str, &str)]) -> Event<'static> {
    let mut event = Event::default();
    let mut t: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in tags {
        t.insert((*k).to_string(), (*v).to_string());
    }
    event.tags = t;
    event
}

/// Drive an envelope-capturing Sentry client through a sequence of events
/// and return how many made it past `before_send`.
///
/// `sentry::init` mutates the process-global Sentry hub; Cargo runs integration
/// test functions in parallel threads by default, so two `count_captured` calls
/// would otherwise race on the global hub and one test's `capture_event` could
/// land in another test's transport. Serialize the critical section here rather
/// than imposing `--test-threads=1` on the whole binary.
fn count_captured(events: Vec<Event<'static>>) -> usize {
    static SENTRY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SENTRY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let transport = sentry::test::TestTransport::new();
    let transport_for_factory = transport.clone();
    let options = sentry::ClientOptions {
        dsn: Some(
            "https://public@sentry.example.com/1"
                .parse()
                .expect("dsn parses"),
        ),
        // Same filter shape the real binary installs in main.rs.
        before_send: Some(Arc::new(|event| {
            if is_transient_provider_http_failure(&event) {
                None
            } else {
                Some(event)
            }
        })),
        transport: Some(Arc::new(move |_opts: &sentry::ClientOptions| {
            transport_for_factory.clone() as Arc<dyn sentry::Transport>
        })),
        sample_rate: 1.0,
        ..sentry::ClientOptions::default()
    };
    let _sentry_guard = sentry::init(options);
    for event in events {
        sentry::capture_event(event);
    }
    sentry::Hub::current()
        .client()
        .map(|c| c.flush(Some(std::time::Duration::from_secs(2))));
    transport.fetch_and_clear_envelopes().len()
}

#[test]
fn drops_per_attempt_429_503_504_408_502() {
    // Each of these matches the tag shape `ops::api_error` sets when a
    // transient upstream status returns. With the filter installed in
    // before_send, none should leak through to the transport.
    let events = ["429", "503", "504", "408", "502"]
        .into_iter()
        .map(|status| {
            event_with_tags(&[
                ("domain", "llm_provider"),
                ("failure", "non_2xx"),
                ("status", status),
            ])
        })
        .collect();
    assert_eq!(
        count_captured(events),
        0,
        "transient per-attempt failures must be filtered in before_send"
    );
}

#[test]
fn keeps_permanent_failures() {
    // 4xx auth / not-found / etc. and 500 internal errors are actionable —
    // they must reach Sentry exactly as before.
    let events = ["400", "401", "403", "404", "500"]
        .into_iter()
        .map(|status| {
            event_with_tags(&[
                ("domain", "llm_provider"),
                ("failure", "non_2xx"),
                ("status", status),
            ])
        })
        .collect();
    assert_eq!(
        count_captured(events),
        5,
        "permanent provider failures must reach Sentry"
    );
}

#[test]
fn keeps_aggregate_all_exhausted_event() {
    // The reliable_chat layer fires a single aggregate
    // `failure=all_exhausted` event when every provider/model has been
    // tried. That's the cascade signal we want — only the per-attempt
    // noise gets dropped.
    let event = event_with_tags(&[
        ("domain", "llm_provider"),
        ("failure", "all_exhausted"),
        ("model", "claude-haiku-4-5-20251001"),
        ("attempts", "12"),
    ]);
    assert_eq!(
        count_captured(vec![event]),
        1,
        "aggregate all_exhausted event must surface for genuine outages"
    );
}

#[test]
fn keeps_event_missing_status_tag() {
    // Belt-and-suspenders: an event with `failure=non_2xx` but no `status`
    // tag (e.g. a future call site forgets to attach one) must NOT be
    // silently dropped — we'd rather see it and fix the tag emission.
    let event = event_with_tags(&[("domain", "llm_provider"), ("failure", "non_2xx")]);
    assert_eq!(
        count_captured(vec![event]),
        1,
        "event without status tag must not be silently dropped"
    );
}
