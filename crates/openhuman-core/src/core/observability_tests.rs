use super::*;
use crate::inference::registry::{
    register_backend_error_classifier, BackendErrorClassification, BackendErrorKind,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(feature = "crash-reporting")]
fn event_with_tags(pairs: &[(&str, &str)]) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (k, v) in pairs {
        tags.insert((*k).to_string(), (*v).to_string());
    }
    event.tags = tags;
    event
}

#[cfg(feature = "crash-reporting")]
fn event_with_message(msg: &str) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    event.message = Some(msg.to_string());
    event
}

#[cfg(feature = "crash-reporting")]
fn channel_message_404_event(method: &str) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    event.tags.insert("domain".into(), "backend_api".into());
    event.tags.insert("failure".into(), "non_2xx".into());
    event.tags.insert("status".into(), "404".into());
    event.tags.insert("method".into(), method.into());
    event.message = Some(
        "PATCH /channels/telegram/messages/1103 failed (404); response_body_len=172".to_string(),
    );
    event
}

fn managed_body(status: &str, code: &str) -> String {
    let body = format!(
        "OpenHuman API error ({status}): {{\"error\":{{\"errorCode\":\"{code}\",\"message\":\"x\"}}}}"
    );
    register_backend_classifier_fixture(
        &body,
        BackendErrorClassification {
            provider_owned: true,
            backend_owned: true,
            reportable: false,
            token: Some("fixture-owned".to_string()),
            kind: Some(BackendErrorKind::Internal),
            malformed_bad_request: false,
        },
    );
    body
}

/// Installs exact-message metadata for core tests of the generic classifier
/// hook. The tests supply ownership and reportability directly; this fixture
/// deliberately contains no hosted token parser or vocabulary.
fn register_backend_classifier_fixture(message: &str, classification: BackendErrorClassification) {
    static FIXTURES: OnceLock<Arc<Mutex<HashMap<String, BackendErrorClassification>>>> =
        OnceLock::new();
    let fixtures = Arc::clone(FIXTURES.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))));
    let classifier_fixtures = Arc::clone(&fixtures);
    register_backend_error_classifier(Arc::new(move |_provider, error| {
        classifier_fixtures
            .lock()
            .expect("classifier fixture lock")
            .get(error)
            .cloned()
            .unwrap_or(BackendErrorClassification {
                provider_owned: false,
                backend_owned: false,
                reportable: true,
                token: None,
                kind: None,
                malformed_bad_request: false,
            })
    }));
    fixtures
        .lock()
        .expect("classifier fixture lock")
        .insert(message.to_string(), classification);
}

fn reportable_fixture(
    kind: BackendErrorKind,
    malformed_bad_request: bool,
) -> BackendErrorClassification {
    BackendErrorClassification {
        provider_owned: true,
        backend_owned: true,
        reportable: true,
        token: Some("fixture-reportable".to_string()),
        kind: Some(kind),
        malformed_bad_request,
    }
}

#[cfg(feature = "crash-reporting")]
fn event_with_tags_and_message(
    pairs: &[(&str, &str)],
    message: &str,
) -> sentry::protocol::Event<'static> {
    let mut event = event_with_tags(pairs);
    event.message = Some(message.to_string());
    event
}

#[cfg(feature = "crash-reporting")]
fn event_with_exception_value(value: &str) -> sentry::protocol::Event<'static> {
    let mut event = sentry::protocol::Event::default();
    event.exception = vec![sentry::protocol::Exception {
        value: Some(value.to_string()),
        ..Default::default()
    }]
    .into();
    event
}

#[path = "observability_crash_filter_integrations_tests.rs"]
mod crash_filter_integrations;
#[path = "observability_crash_filter_messages_tests.rs"]
mod crash_filter_messages;
#[path = "observability_error_classification_core_tests.rs"]
mod error_classification_core;
#[path = "observability_error_classification_network_tests.rs"]
mod error_classification_network;
#[path = "observability_error_classification_provider_tests.rs"]
mod error_classification_provider;
#[path = "observability_error_classification_session_tests.rs"]
mod error_classification_session;
#[path = "observability_error_classification_user_state_tests.rs"]
mod error_classification_user_state;

/// Log output of `f` under a plain fmt subscriber.
fn captured_logs(f: impl FnOnce()) -> String {
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let out = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    out
}

#[test]
fn expected_and_reported_errors_are_logged_scrubbed() {
    let msg = "Embedding API error (429 Too Many Requests): Bearer sekrit-value rejected";
    assert!(
        expected_error_kind(msg).is_some(),
        "fixture must take the expected-error path"
    );
    let expected = captured_logs(|| report_error_or_expected(&msg, "d", "o", &[]));
    assert!(expected.contains("[REDACTED]"), "{expected}");
    assert!(!expected.contains("sekrit"), "{expected}");

    let reported = captured_logs(|| report_error(&"boom Bearer sekrit-value", "d", "o", &[]));
    assert!(
        !reported.is_empty() && !reported.contains("sekrit"),
        "{reported}"
    );
}

#[test]
fn a_backend_error_code_survives_a_secret_in_the_message() {
    let body = "OpenHuman API error (400): {\"error\":{\"message\":\"bad Bearer sekrit-value\",\"errorCode\":\"x\"}}";
    register_backend_classifier_fixture(
        body,
        BackendErrorClassification {
            provider_owned: true,
            backend_owned: true,
            reportable: false,
            token: Some("fixture-owned".to_string()),
            kind: Some(BackendErrorKind::Internal),
            malformed_bad_request: false,
        },
    );
    assert_eq!(
        expected_error_kind(body),
        Some(ExpectedErrorKind::BackendErrorCodeOwned)
    );
    let logs = captured_logs(|| report_error_or_expected(&body, "d", "o", &[]));
    assert!(logs.contains("errorCode=fixture-owned"), "{logs}");
    assert!(!logs.contains("sekrit"), "{logs}");

    // An errorCode that is itself secret-shaped is scrubbed too.
    let leaky = "OpenHuman API error (400): {\"error\":{\"errorCode\":\"code-x\"}}";
    register_backend_classifier_fixture(
        leaky,
        BackendErrorClassification {
            provider_owned: true,
            backend_owned: true,
            reportable: false,
            token: Some("sk-proj-LEAKEDLEAKEDLEAKED".to_string()),
            kind: Some(BackendErrorKind::Internal),
            malformed_bad_request: false,
        },
    );
    let logs = captured_logs(|| report_error_or_expected(&leaky, "d", "o", &[]));
    assert!(logs.contains("errorCode=[REDACTED]"), "{logs}");
    assert!(!logs.contains("LEAKED"), "{logs}");
}
