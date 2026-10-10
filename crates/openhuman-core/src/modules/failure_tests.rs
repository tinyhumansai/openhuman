use super::*;

#[cfg(feature = "crash-reporting")]
#[test]
fn terminal_reports_are_sanitized_and_deduplicated() {
    let record = crate::modules::registry::find("tinybox").unwrap();
    let events = sentry::test::with_captured_events(|| {
        sentry::with_scope(
            |scope| {
                scope.set_tag("unsafe_path", "/home/private-user/document.txt");
                scope.set_extra(
                    "unsafe_payload",
                    "private-test-token and user content".into(),
                );
            },
            || {
                report(record, Reason::ModuleFault);
                report(record, Reason::ModuleFault);
            },
        );
    });
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    let captured = format!("{event:?}");
    assert!(!captured.contains("private-test-token"));
    assert!(!captured.contains("/home/private-user"));
    assert!(!event.tags.contains_key("unsafe_path"));
    assert!(event.extra.is_empty());
    assert_eq!(
        event.message.as_deref(),
        Some("loadable module operation failed")
    );
    for (key, value) in [
        ("module", record.id),
        ("version", record.version),
        ("stage", "execution"),
        ("reason_code", "module_fault"),
    ] {
        assert_eq!(event.tags.get(key).map(String::as_str), Some(value));
    }
    assert_eq!(
        event.tags.get("platform"),
        Some(&format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    );
}

#[test]
fn reason_codes_and_stages_are_closed_vocabulary() {
    for reason in [
        Reason::UnknownModule,
        Reason::Disabled,
        Reason::LoaderDisabled,
        Reason::ResolutionFailed,
        Reason::IncompatibleContract,
        Reason::TransportFailed,
        Reason::ModuleFault,
        Reason::ModuleUnavailable,
    ] {
        assert!(reason
            .code()
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_'));
        assert!(reason.stage().chars().all(|c| c.is_ascii_lowercase()));
    }
}

#[cfg(feature = "crash-reporting")]
#[test]
fn a_pre_core_report_without_a_sentry_client_can_be_captured_later() {
    let record = crate::modules::registry::find("tinyhosts").unwrap();
    let hub = std::sync::Arc::new(sentry::Hub::new_from_top(sentry::Hub::current()));
    hub.bind_client(None);
    sentry::Hub::run(hub, || report(record, Reason::IncompatibleContract));
    let events = sentry::test::with_captured_events(|| {
        report(record, Reason::IncompatibleContract);
        report(record, Reason::IncompatibleContract);
    });
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        events[0].tags.get("module").map(String::as_str),
        Some("tinyhosts")
    );
}
