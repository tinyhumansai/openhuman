use super::*;

#[test]
fn defaults_route_through_the_shipped_queues_in_storage() {
    let c = InboundConfig::default();
    assert!(c.enabled);
    assert_eq!(c.queues_path, None);
    assert_eq!(c.pipeline_path, None);
    assert_eq!(c.queue_url, None);
    assert_eq!(c.triage_timeout_ms, DEFAULT_TRIAGE_TIMEOUT_MS);
}

#[test]
fn a_partial_section_keeps_the_other_defaults() {
    let c: InboundConfig =
        toml::from_str("enabled = false\nqueues_path = \"/etc/oh/queues.json\"").unwrap();
    assert!(!c.enabled);
    assert_eq!(c.queues_path, Some(PathBuf::from("/etc/oh/queues.json")));
    assert_eq!(c.triage_timeout_ms, DEFAULT_TRIAGE_TIMEOUT_MS);
}

#[test]
fn the_triage_timeout_stays_under_the_module_callback_deadline() {
    let mut c = InboundConfig::default();
    assert_eq!(
        c.triage_timeout().as_millis(),
        u128::from(MAX_TRIAGE_TIMEOUT_MS)
    );
    c.triage_timeout_ms = 90_000;
    assert_eq!(
        c.triage_timeout().as_millis(),
        u128::from(MAX_TRIAGE_TIMEOUT_MS)
    );
    c.triage_timeout_ms = 0;
    assert_eq!(c.triage_timeout().as_millis(), 1);
    c.triage_timeout_ms = 5_000;
    assert_eq!(c.triage_timeout().as_millis(), 5_000);
}

#[test]
fn debug_redacts_the_broker_credentials() {
    let c = InboundConfig {
        queue_url: Some("amqp://user:hunter2@broker.internal/vhost".into()),
        ..InboundConfig::default()
    };
    let shown = format!("{c:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("broker.internal"), "{shown}");
}
